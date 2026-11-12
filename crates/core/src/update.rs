//! Updates: `latest.json` in the latest GitHub release names a package per platform, and minisign
//! signs both. A package downloads to a temp file, is verified there and installed from it.

use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::Ordering;
use std::time::Duration;

use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::Deserialize;
use tempfile::TempPath;

use crate::error::{Error, Result};
use crate::state::Cancel;

#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

/// The release key's public half: the second line of its `minisign.pub`.
pub const PUBLIC_KEY: &str = "RWQA2ncifHAARGR4m3nVLn/10nR/FZjkpUc8ObrB5Ca4FS9UyD/QIzeZ";
pub const LATEST: &str = "https://github.com/YohoCX/detached/releases/latest/download";
pub const RELEASES: &str = "https://github.com/YohoCX/detached/releases/latest";

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    pub url: String,
    /// The `.minisig` file's text.
    pub signature: String,
}

/// The release `{base}/latest.json` has for this platform, when it is newer than `current` and the
/// file's signature by `key` holds.
pub fn check(base: &str, current: &str, key: &str) -> Result<Option<Release>> {
    let get = |file| get(&format!("{base}/{file}"));
    let json = get("latest.json")?;
    let signature = get("latest.json.minisig")?;
    verify(&json, &String::from_utf8_lossy(&signature), key)?;
    release(&json, &platform(), current)
}

/// This build's key in `latest.json`'s platforms.
fn platform() -> String {
    format!("{}-{}", env::consts::OS, env::consts::ARCH)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// Bytes so far, and the length when the server sent one.
    Received(u64, Option<u64>),
    Verifying,
}

const PACKAGE: &str = ".detached-package";

/// The release's package in a temp file in `dir` once its signature by `key` holds. The file goes
/// with the result, and at once on an error or a cancel.
pub fn download(
    release: &Release,
    key: &str,
    dir: &Path,
    cancel: &Cancel,
    progress: &dyn Fn(Progress),
) -> Result<TempPath> {
    // A quit skips the TempPath's drop, so a quit's package is still here.
    for e in fs::read_dir(dir)?.flatten() {
        if e.file_name().to_string_lossy().starts_with(PACKAGE) {
            let _ = fs::remove_file(e.path());
        }
    }
    let mut file = tempfile::Builder::new().prefix(PACKAGE).tempfile_in(dir)?;
    let mut response = call(&release.url)?;
    let total = response.body().content_length();
    let mut body = response.body_mut().as_reader();
    let (mut received, mut chunk) = (0, vec![0; 1 << 16]);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        match body.read(&mut chunk).map_err(|e| fail(&release.url, &e))? {
            0 => break,
            n => {
                file.write_all(&chunk[..n])?;
                received += n as u64;
                progress(Progress::Received(received, total));
            }
        }
    }
    progress(Progress::Verifying);
    file.rewind()?;
    verify_file(file.as_file_mut(), &release.signature, key)?;
    Ok(file.into_temp_path())
}

fn release(json: &[u8], platform: &str, current: &str) -> Result<Option<Release>> {
    #[derive(Deserialize)]
    struct Latest {
        version: Version,
        platforms: HashMap<String, Package>,
    }
    #[derive(Deserialize)]
    struct Package {
        url: String,
        signature: String,
    }
    let mut latest: Latest = serde_json::from_slice(json)?;
    let current = Version::parse(current).map_err(|e| Error::Other(e.to_string()))?;
    let package = latest.platforms.remove(platform);
    Ok(package
        .filter(|_| latest.version > current)
        .map(|p| Release {
            version: latest.version,
            url: p.url,
            signature: p.signature,
        }))
}

fn fail(url: &str, e: &dyn std::fmt::Display) -> Error {
    Error::Other(format!("{url}: {e}"))
}

fn call(url: &str) -> Result<ureq::http::Response<ureq::Body>> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .timeout_per_read(Some(Duration::from_secs(30)));
    ureq::Agent::new_with_config(config.build())
        .get(url)
        .call()
        .map_err(|e| fail(url, &e))
}

fn get(url: &str) -> Result<Vec<u8>> {
    let mut data = vec![];
    call(url)?
        .body_mut()
        .as_reader()
        .read_to_end(&mut data)
        .map_err(|e| fail(url, &e))?;
    Ok(data)
}

fn keys(signature: &str, key: &str) -> Result<(PublicKey, Signature)> {
    Ok((
        PublicKey::from_base64(key).map_err(bad)?,
        Signature::decode(signature).map_err(bad)?,
    ))
}

fn bad(e: minisign_verify::Error) -> Error {
    Error::Other(format!("signature: {e}"))
}

fn verify(data: &[u8], signature: &str, key: &str) -> Result<()> {
    let (key, signature) = keys(signature, key)?;
    key.verify(data, &signature, false).map_err(bad)
}

fn verify_file(file: &mut File, signature: &str, key: &str) -> Result<()> {
    let (key, signature) = keys(signature, key)?;
    let mut stream = key.verify_stream(&signature).map_err(bad)?;
    let mut chunk = vec![0; 1 << 16];
    loop {
        match file.read(&mut chunk)? {
            0 => return stream.finalize().map_err(bad),
            n => stream.update(&chunk[..n]),
        }
    }
}

/// How this copy was installed, which decides how it updates.
#[derive(Clone, Debug, PartialEq)]
pub enum Install {
    /// The `.app` bundle.
    Mac(PathBuf),
    /// By the NSIS installer, which an update runs again.
    Windows,
    AppImage(PathBuf),
    /// By the `.deb`, which only the package manager updates.
    Deb,
}

impl Install {
    /// None when not run from a release package: cargo runs, `make app`, tests.
    pub fn current() -> Option<Install> {
        let exe = env::current_exe().ok()?;
        let dir = exe.parent()?;
        if cfg!(target_os = "macos") {
            let app = dir.parent()?.parent()?;
            let bundle = dir.ends_with("Contents/MacOS") && app.extension()? == "app";
            return bundle.then(|| Install::Mac(app.into()));
        }
        if cfg!(windows) {
            return dir
                .join("uninstall.exe")
                .exists()
                .then_some(Install::Windows);
        }
        // Children of another AppImage inherit its variables; ours runs from its own mount.
        let mounted = env::var_os("APPDIR").is_some_and(|d| exe.starts_with(d));
        match env::var_os("APPIMAGE") {
            Some(image) if mounted => Some(Install::AppImage(image.into())),
            _ => (exe == Path::new("/usr/bin/detached")).then_some(Install::Deb),
        }
    }

    /// Puts the verified package in place and has the new version start once this process exits,
    /// which must be at once: the old bundle is gone when this returns.
    pub fn apply(&self, package: &Path) -> Result<()> {
        match self {
            Install::Mac(app) => {
                replace_app(app, package)?;
                relaunch(&["open".as_ref(), app.as_os_str()])
            }
            Install::AppImage(image) => {
                replace_file(image, package)?;
                relaunch(&[image.as_os_str()])
            }
            Install::Windows => {
                let setup = env::temp_dir().join("Detached-setup.exe");
                fs::copy(package, &setup)?;
                // The installer stops this process and starts the new one (/R); /NS leaves the
                // shortcuts as the user left them.
                Command::new(setup).args(["/S", "/NS", "/R"]).spawn()?;
                Ok(())
            }
            Install::Deb => Err(Error::Other("the package manager updates a deb".into())),
        }
    }
}

/// Swaps `app` for the bundle in `zip` after the checks install.sh makes; `app` comes back if the
/// swap fails.
fn replace_app(app: &Path, zip: &Path) -> Result<()> {
    let dir = app
        .parent()
        .ok_or_else(|| Error::Other("a bundle lies in a folder".into()))?;
    let tmp = tempfile::Builder::new()
        .prefix(".detached-update")
        .tempdir_in(dir)?;
    let [new, old] = ["Detached.app", "old.app"].map(|n| tmp.path().join(n));
    run(Command::new("ditto")
        .args(["-x", "-k"])
        .arg(zip)
        .arg(tmp.path()))?;
    run(Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&new))?;
    fs::rename(app, &old)?;
    fs::rename(&new, app).inspect_err(|_| {
        fs::rename(&old, app).ok();
    })?;
    Ok(())
}

/// Copies `package` next to `path` and renames it over `path`, executable.
fn replace_file(path: &Path, package: &Path) -> Result<()> {
    let mut new = path.as_os_str().to_owned();
    new.push(".new");
    fs::copy(package, &new)?;
    #[cfg(unix)]
    fs::set_permissions(&new, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    fs::rename(&new, path)?;
    Ok(())
}

/// Runs `cmd` once this process has exited.
fn relaunch(cmd: &[&OsStr]) -> Result<()> {
    let wait = r#"while kill -0 "$0" 2>/dev/null; do sleep 0.2; done; exec "$@""#;
    Command::new("sh")
        .args(["-c", wait])
        .arg(process::id().to_string())
        .args(cmd)
        .spawn()?;
    Ok(())
}

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output()?;
    if out.status.success() {
        return Ok(());
    }
    Err(Error::Other(
        String::from_utf8_lossy(&out.stderr).trim().into(),
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::testutil::{Server, keys};
    use super::*;

    #[test]
    fn only_a_newer_version_is_offered() {
        let newer = |version: &str, current| {
            let package = serde_json::json!({"url": "u", "signature": "s"});
            let json =
                serde_json::json!({"version": version, "platforms": {"linux-x86_64": package}});
            release(json.to_string().as_bytes(), "linux-x86_64", current).map(|r| r.is_some())
        };
        assert!(newer("0.10.0", "0.9.0").unwrap());
        assert!(!newer("0.3.0", "0.3.0").unwrap());
        assert!(!newer("0.2.9", "0.3.0").unwrap());
        assert!(newer("v1.0.0", "0.3.0").is_err());
    }

    #[test]
    fn a_signature_holds_for_its_bytes_and_key_only() {
        let (key, sign) = keys();
        let signature = sign(b"package");
        assert!(verify(b"package", &signature, &key).is_ok());
        assert!(verify(b"packagf", &signature, &key).is_err());
        assert!(verify(b"package", &signature, &keys().0).is_err());
        assert!(verify(b"package", "garbage", &key).is_err());
        assert!(verify(b"package", &signature, PUBLIC_KEY).is_err());
    }

    /// Downloads `release` into `dir` with no cancel, recording its progress.
    fn fetch(release: &Release, key: &str, dir: &Path) -> (Result<TempPath>, Vec<Progress>) {
        let seen = RefCell::new(vec![]);
        let file = download(release, key, dir, &Cancel::default(), &|p| {
            seen.borrow_mut().push(p)
        });
        (file, seen.into_inner())
    }

    #[test]
    fn a_newer_release_is_offered_downloaded_and_verified() {
        let (key, sign) = keys();
        let package = b"the new app".to_vec();
        let server = Server::start(&sign, "9.9.9", &package, &sign(&package));
        let release = check(&server.base, "0.3.0", &key)
            .unwrap()
            .expect("offered");
        assert_eq!(release.version, Version::new(9, 9, 9));
        let dir = tempfile::tempdir().unwrap();
        let file = fetch(&release, &key, dir.path()).0.unwrap();
        assert_eq!(fs::read(&file).unwrap(), package);
        drop(file);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_download_removes_the_package_a_quit_left() {
        let (key, sign) = keys();
        let package = b"the new app".to_vec();
        let server = Server::start(&sign, "9.9.9", &package, &sign(&package));
        let release = check(&server.base, "0.3.0", &key).unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        fetch(&release, &key, dir.path()).0.unwrap().keep().unwrap();
        fs::write(dir.path().join("other"), "").unwrap();
        let file = fetch(&release, &key, dir.path()).0.unwrap();
        let mut left: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        left.sort();
        let mut want = vec![file.to_path_buf(), dir.path().join("other")];
        want.sort();
        assert_eq!(left, want);
    }

    #[test]
    fn the_running_version_is_not_offered() {
        let (key, sign) = keys();
        let server = Server::start(&sign, "0.3.0", b"", "");
        assert_eq!(check(&server.base, "0.3.0", &key).unwrap(), None);
    }

    #[test]
    fn a_missing_file_or_a_bad_signature_is_an_error_and_leaves_no_file() {
        let (key, sign) = keys();
        let server = Server::start(&sign, "9.9.9", b"tampered", &sign(b"signed"));
        assert!(check(&format!("{}/missing", server.base), "0.3.0", &key).is_err());
        let release = check(&server.base, "0.3.0", &key).unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let err = fetch(&release, &key, dir.path()).0.unwrap_err();
        assert!(err.to_string().starts_with("signature: "), "{err}");
        let missing = Release {
            url: format!("{}/missing", server.base),
            ..release
        };
        assert!(fetch(&missing, &key, dir.path()).0.is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// Its URL and signature would otherwise offer any signed package: an older release's, or
    /// another platform's.
    #[test]
    fn a_latest_json_not_signed_by_the_key_is_not_offered() {
        let (key, sign) = keys();
        let package = b"an older app".to_vec();
        let server = Server::start(keys().1, "9.9.9", &package, &sign(&package));
        let err = check(&server.base, "0.3.0", &key).unwrap_err();
        assert!(err.to_string().starts_with("signature: "), "{err}");
    }

    #[test]
    fn a_cancelled_download_stops_and_leaves_no_file() {
        let (key, sign) = keys();
        let server = Server::start(&sign, "9.9.9", b"package", &sign(b"package"));
        let release = check(&server.base, "0.3.0", &key).unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cancel = Cancel::new(true.into());
        let done = download(&release, &key, dir.path(), &cancel, &|_| {});
        assert!(matches!(done, Err(Error::Cancelled)));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn progress_rises_to_the_length_the_server_sends_then_verifies() {
        let (key, sign) = keys();
        let package = vec![7; 11 << 20];
        let server = Server::start(&sign, "9.9.9", &package, &sign(&package));
        let release = check(&server.base, "0.3.0", &key).unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (file, seen) = fetch(&release, &key, dir.path());
        assert_eq!(
            fs::metadata(file.unwrap()).unwrap().len(),
            package.len() as u64
        );
        let len = package.len() as u64;
        assert_eq!(seen.last(), Some(&Progress::Verifying));
        let received: Vec<_> = seen[..seen.len() - 1]
            .iter()
            .map(|p| match p {
                Progress::Received(n, total) => (*n, *total),
                Progress::Verifying => panic!("verifying before the end"),
            })
            .collect();
        assert!(received.len() > 1);
        assert!(received.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(received.iter().all(|r| r.1 == Some(len)));
        assert_eq!(received.last().unwrap().0, len);
    }

    #[test]
    fn without_a_length_progress_counts_bytes_alone() {
        let (key, sign) = keys();
        let package = vec![7; 1 << 20];
        let server = Server::start(&sign, "9.9.9", &package, &sign(&package));
        server.without_length();
        let release = check(&server.base, "0.3.0", &key).unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (file, seen) = fetch(&release, &key, dir.path());
        file.unwrap();
        let len = package.len() as u64;
        assert_eq!(seen[seen.len() - 2], Progress::Received(len, None));
        assert!(
            seen.iter()
                .all(|p| !matches!(p, Progress::Received(_, Some(_))))
        );
    }

    /// scripts/latest-json.sh, which the release job runs on Linux.
    #[cfg(unix)]
    #[test]
    fn the_release_jobs_latest_json_names_each_platforms_package() {
        let (key, sign) = keys();
        let dir = tempfile::tempdir().unwrap();
        let packages = [
            ("macos-aarch64", "Detached-macos-arm64.zip"),
            ("macos-x86_64", "Detached-macos-x64.zip"),
            ("windows-x86_64", "Detached-windows-x64-setup.exe"),
            ("linux-x86_64", "Detached-linux-x86_64.AppImage"),
        ];
        for (_, file) in packages {
            fs::write(
                dir.path().join(format!("{file}.minisig")),
                sign(file.as_bytes()),
            )
            .unwrap();
        }
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/latest-json.sh");
        let out = Command::new("sh")
            .args([script, "0.3.0", "https://x"])
            .arg(dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let json = &out.stdout;
        for (platform, file) in packages {
            let release = release(json, platform, "0.2.9").unwrap().unwrap();
            assert_eq!(release.url, format!("https://x/{file}"));
            verify(file.as_bytes(), &release.signature, &key).unwrap();
        }
        if cfg!(target_os = "macos") {
            let arch = if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x64"
            };
            let mine = release(json, &platform(), "0.2.9").unwrap().unwrap();
            assert_eq!(mine.url, format!("https://x/Detached-macos-{arch}.zip"));
        }
        assert_eq!(release(json, "linux-aarch64", "0.2.9").unwrap(), None);
        assert!(release(&json[..40], "linux-x86_64", "0.2.9").is_err());
    }

    #[test]
    fn a_cargo_built_binary_is_not_an_install() {
        assert_eq!(Install::current(), None);
    }

    #[cfg(unix)]
    #[test]
    fn an_appimage_is_replaced_in_place_and_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (image, package) = (dir.path().join("Detached.AppImage"), dir.path().join("p"));
        fs::write(&image, "old").unwrap();
        fs::write(&package, "new").unwrap();
        replace_file(&image, &package).unwrap();
        assert_eq!(fs::read(&image).unwrap(), b"new");
        let mode = fs::metadata(&image).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
