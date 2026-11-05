//! Auto-update: `latest.json` in the latest GitHub release names a package per platform, and
//! minisign signs both; the status bar offers a newer release, and a click downloads, verifies and
//! installs it.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::{env, fs, process};

use detached_core::error::{Error, Result};
use detached_core::state::Cancel;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, FontWeight, Global, IntoElement, ParentElement, Role, StatefulInteractiveElement, Styled,
    prelude::FluentBuilder, px,
};
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::Deserialize;

use crate::blocking;
use crate::store::Store;
use crate::ui::{self, theme};

/// The release key's public half: the second line of its `minisign.pub`.
pub const PUBLIC_KEY: &str = "";
const LATEST: &str = "https://github.com/YohoCX/detached/releases/latest/download";
const RELEASES: &str = "https://github.com/YohoCX/detached/releases/latest";
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

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
    let get = |file| get(&format!("{base}/{file}"), &Cancel::default());
    let json = get("latest.json")?;
    let signature = get("latest.json.minisig")?;
    verify(&json, &String::from_utf8_lossy(&signature), key)?;
    let platform = format!("{}-{}", env::consts::OS, env::consts::ARCH);
    release(&json, &platform, current)
}

/// The release's package, once its signature by `key` holds.
pub fn download(release: &Release, key: &str, cancel: &Cancel) -> Result<Vec<u8>> {
    let package = get(&release.url, cancel)?;
    verify(&package, &release.signature, key)?;
    Ok(package)
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

fn get(url: &str, cancel: &Cancel) -> Result<Vec<u8>> {
    let fail = |e: &dyn std::fmt::Display| Error::Other(format!("{url}: {e}"));
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .timeout_per_read(Some(Duration::from_secs(30)));
    let mut response = ureq::Agent::new_with_config(config.build())
        .get(url)
        .call()
        .map_err(|e| fail(&e))?;
    // ureq stops reading a body at 10 MB unless told otherwise; a package is larger.
    let mut body = response.body_mut().with_config().limit(u64::MAX).reader();
    let (mut data, mut chunk) = (vec![], vec![0; 1 << 16]);
    while !cancel.load(Ordering::Relaxed) {
        match body.read(&mut chunk).map_err(|e| fail(&e))? {
            0 => return Ok(data),
            n => data.extend_from_slice(&chunk[..n]),
        }
    }
    Err(Error::Cancelled)
}

fn verify(data: &[u8], signature: &str, key: &str) -> Result<()> {
    let fail = |e: minisign_verify::Error| Error::Other(format!("signature: {e}"));
    let key = PublicKey::from_base64(key).map_err(fail)?;
    let signature = Signature::decode(signature).map_err(fail)?;
    key.verify(data, &signature, false).map_err(fail)
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
    fn current() -> Option<Install> {
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

    /// Puts the verified package in place and has the new version start once this process exits.
    fn apply(&self, package: &[u8]) -> Result<()> {
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
                fs::write(&setup, package)?;
                // The installer stops this process and starts the new one (/R); /NS leaves the
                // shortcuts as the user left them.
                Command::new(setup).args(["/S", "/NS", "/R"]).spawn()?;
                Ok(())
            }
            Install::Deb => unreachable!("a deb is updated by the package manager"),
        }
    }
}

/// Swaps `app` for the bundle in `zip` after the checks install.sh makes; `app` comes back if the
/// swap fails.
fn replace_app(app: &Path, zip: &[u8]) -> Result<()> {
    let dir = app.parent().expect("a bundle lies in a folder");
    let tmp = tempfile::Builder::new()
        .prefix(".detached-update")
        .tempdir_in(dir)?;
    let [file, new, old] = ["Detached.zip", "Detached.app", "old.app"].map(|n| tmp.path().join(n));
    fs::write(&file, zip)?;
    run(Command::new("ditto")
        .args(["-x", "-k"])
        .arg(&file)
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

/// Writes `package` next to `path` and renames it over `path`, executable.
fn replace_file(path: &Path, package: &[u8]) -> Result<()> {
    let mut new = path.as_os_str().to_owned();
    new.push(".new");
    fs::write(&new, package)?;
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

/// The offered update; set when a check finds one.
pub struct Update {
    pub install: Install,
    pub release: Release,
    /// The running install's cancel flag.
    pub installing: Option<Cancel>,
}

impl Global for Update {}

/// When run from a release package, checks at launch and then daily until a release is offered.
pub fn init(cx: &mut App) {
    let Some(install) = Install::current() else {
        return;
    };
    cx.spawn(async move |cx| {
        loop {
            let found = cx.update(|cx| {
                blocking::run(cx, || check(LATEST, env!("CARGO_PKG_VERSION"), PUBLIC_KEY))
            });
            // Failures stay silent: offline, rate limited, or a release without latest.json.
            if let Ok(Some(release)) = found.await {
                let update = Update {
                    install,
                    release,
                    installing: None,
                };
                return cx.update(|cx| cx.set_global(update));
            }
            cx.background_executor().timer(DAY).await;
        }
    })
    .detach();
}

/// The status bar's item for an offered update, with a spinner while it installs; a click then
/// cancels.
pub fn item(cx: &App) -> Option<impl IntoElement + use<>> {
    let u = cx.try_global::<Update>()?;
    let t = theme(cx);
    let label = match u.install {
        Install::Deb => format!("Detached {} is out", u.release.version),
        _ => format!("Update to {}", u.release.version),
    };
    Some(
        ui::pressable("update")
            .test_support()
            .gap(px(4.))
            .role(Role::Button)
            .aria_label(label.clone())
            .text_color(t.accent)
            .font_weight(FontWeight::SEMIBOLD)
            .when(u.installing.is_some(), |d| {
                d.child(ui::spinner(t.accent))
                    .tooltip(|window, cx| Tooltip::new("Cancel").build(window, cx))
            })
            .child(label)
            .on_click(|_, _, cx| install(cx)),
    )
}

fn install(cx: &mut App) {
    let u = cx.global_mut::<Update>();
    // Reset at once: a stalled download sees the flag only at its next read.
    if let Some(cancel) = u.installing.take() {
        return cancel.store(true, Ordering::Relaxed);
    }
    if u.install == Install::Deb {
        return cx.open_url(RELEASES);
    }
    let cancel = Cancel::default();
    u.installing = Some(cancel.clone());
    let (install, release, c) = (u.install.clone(), u.release.clone(), cancel.clone());
    let job = blocking::run(cx, move || {
        install.apply(&download(&release, PUBLIC_KEY, &c)?)
    });
    cx.spawn(async move |cx| {
        let done = job.await;
        cx.update(|cx| match done {
            Ok(()) => cx.quit(),
            Err(_) if cancel.load(Ordering::Relaxed) => {}
            Err(e) => {
                cx.global_mut::<Update>().installing = None;
                let text = format!("Update failed: {e}");
                Store::global(cx).update(cx, |s, cx| s.say(text, true, cx));
            }
        })
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

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

    /// A fresh key as `PUBLIC_KEY` holds one, and a signer that writes `.minisig` text.
    fn keys() -> (String, impl Fn(&[u8]) -> String) {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let key = pair.pk.to_base64();
        let sign = move |data: &[u8]| {
            let signature = minisign::sign(None, &pair.sk, data, None, None).unwrap();
            signature.into_string()
        };
        (key, sign)
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

    /// Serves `latest.json` signed by `sign`, offering `version` for this platform as `/package`
    /// with `signature`, on localhost.
    fn serve(
        sign: impl Fn(&[u8]) -> String,
        version: &str,
        package: Vec<u8>,
        signature: &str,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let platform = format!("{}-{}", env::consts::OS, env::consts::ARCH);
        let entry = serde_json::json!({"url": format!("{base}/package"), "signature": signature});
        let latest = serde_json::json!({"version": version, "platforms": {platform: entry}});
        let latest = latest.to_string().into_bytes();
        let files = HashMap::from([
            (
                "/latest.json.minisig".to_string(),
                sign(&latest).into_bytes(),
            ),
            ("/latest.json".to_string(), latest),
            ("/package".to_string(), package),
        ]);
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                // The whole request is read, or closing the socket could reset the response.
                let head: Vec<String> = BufReader::new(&stream)
                    .lines()
                    .map_while(|l| l.ok())
                    .take_while(|l| !l.is_empty())
                    .collect();
                let path = head.first().and_then(|l| l.split(' ').nth(1));
                let (status, body) = match path.and_then(|p| files.get(p)) {
                    Some(body) => ("200 OK", &body[..]),
                    None => ("404 Not Found", &[][..]),
                };
                let len = body.len();
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(head.as_bytes()).ok();
                stream.write_all(body).ok();
            }
        });
        base
    }

    #[test]
    fn a_newer_release_is_offered_downloaded_and_verified() {
        let (key, sign) = keys();
        let package = b"the new app".to_vec();
        let base = serve(&sign, "9.9.9", package.clone(), &sign(&package));
        let release = check(&base, "0.3.0", &key).unwrap().expect("offered");
        assert_eq!(release.version, Version::new(9, 9, 9));
        assert_eq!(
            download(&release, &key, &Cancel::default()).unwrap(),
            package
        );
    }

    #[test]
    fn the_running_version_is_not_offered() {
        let (key, sign) = keys();
        let base = serve(sign, "0.3.0", vec![], "");
        assert_eq!(check(&base, "0.3.0", &key).unwrap(), None);
    }

    #[test]
    fn a_missing_file_or_a_bad_signature_is_an_error() {
        let (key, sign) = keys();
        let base = serve(&sign, "9.9.9", b"tampered".to_vec(), &sign(b"signed"));
        assert!(check(&format!("{base}/missing"), "0.3.0", &key).is_err());
        let release = check(&base, "0.3.0", &key).unwrap().unwrap();
        let err = download(&release, &key, &Cancel::default()).unwrap_err();
        assert!(err.to_string().starts_with("signature: "), "{err}");
        let missing = Release {
            url: format!("{base}/missing"),
            ..release
        };
        assert!(download(&missing, &key, &Cancel::default()).is_err());
    }

    /// Its URL and signature would otherwise offer any signed package: an older release's, or
    /// another platform's.
    #[test]
    fn a_latest_json_not_signed_by_the_key_is_not_offered() {
        let (key, sign) = keys();
        let package = b"an older app".to_vec();
        let base = serve(keys().1, "9.9.9", package.clone(), &sign(&package));
        let err = check(&base, "0.3.0", &key).unwrap_err();
        assert!(err.to_string().starts_with("signature: "), "{err}");
    }

    #[test]
    fn a_cancelled_download_stops() {
        let (key, sign) = keys();
        let base = serve(&sign, "9.9.9", b"package".to_vec(), &sign(b"package"));
        let release = check(&base, "0.3.0", &key).unwrap().unwrap();
        let cancel = Cancel::new(true.into());
        assert!(matches!(
            download(&release, &key, &cancel),
            Err(Error::Cancelled)
        ));
    }

    /// scripts/latest-json.sh, which the release job runs on Linux.
    #[cfg(unix)]
    #[test]
    fn the_release_jobs_latest_json_names_each_platforms_package() {
        let (key, sign) = keys();
        let dir = tempfile::tempdir().unwrap();
        let packages = [
            ("macos-aarch64", "Detached-macos.zip"),
            ("macos-x86_64", "Detached-macos.zip"),
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
        assert_eq!(release(json, "linux-aarch64", "0.2.9").unwrap(), None);
        assert!(release(&json[..40], "linux-x86_64", "0.2.9").is_err());
    }

    #[test]
    fn a_package_over_10_mb_downloads() {
        let (key, sign) = keys();
        let package = vec![7; 11 << 20];
        let base = serve(&sign, "9.9.9", package.clone(), &sign(&package));
        let release = check(&base, "0.3.0", &key).unwrap().unwrap();
        let len = download(&release, &key, &Cancel::default()).unwrap().len();
        assert_eq!(len, package.len());
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
        let image = dir.path().join("Detached.AppImage");
        fs::write(&image, "old").unwrap();
        replace_file(&image, b"new").unwrap();
        assert_eq!(fs::read(&image).unwrap(), b"new");
        let mode = fs::metadata(&image).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
