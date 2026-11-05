//! Auto-update: `latest.json` in the latest GitHub release names a minisign-signed package per
//! platform; the status bar offers a newer release, and a click downloads, verifies and installs it.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use std::{env, fs, process};

use detached_core::error::{Error, Result};
use gpui_kit::base::TestSupportExt;
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

/// The release `{base}/latest.json` has for this platform, when it is newer than `current`.
pub fn check(base: &str, current: &str) -> Result<Option<Release>> {
    let platform = format!("{}-{}", env::consts::OS, env::consts::ARCH);
    release(&get(&format!("{base}/latest.json"))?, &platform, current)
}

/// The release's package, once its signature by `key` holds.
pub fn download(release: &Release, key: &str) -> Result<Vec<u8>> {
    let package = get(&release.url)?;
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

fn get(url: &str) -> Result<Vec<u8>> {
    let fail = |e: ureq::Error| Error::Other(format!("{url}: {e}"));
    let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600)));
    let mut response = ureq::Agent::new_with_config(config.build())
        .get(url)
        .call()
        .map_err(fail)?;
    // ureq stops reading a body at 10 MB unless told otherwise; a package is larger.
    let body = response.body_mut().with_config().limit(u64::MAX);
    body.read_to_vec().map_err(fail)
}

fn verify(package: &[u8], signature: &str, key: &str) -> Result<()> {
    let fail = |e: minisign_verify::Error| Error::Other(format!("signature: {e}"));
    let key = PublicKey::from_base64(key).map_err(fail)?;
    let signature = Signature::decode(signature).map_err(fail)?;
    key.verify(package, &signature, false).map_err(fail)
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
    pub installing: bool,
}

impl Global for Update {}

/// When run from a release package, checks at launch and then daily until a release is offered.
pub fn init(cx: &mut App) {
    let Some(install) = Install::current() else {
        return;
    };
    cx.spawn(async move |cx| {
        loop {
            let found =
                cx.update(|cx| blocking::run(cx, || check(LATEST, env!("CARGO_PKG_VERSION"))));
            // Failures stay silent: offline, rate limited, or a release without latest.json.
            if let Ok(Some(release)) = found.await {
                let update = Update {
                    install,
                    release,
                    installing: false,
                };
                return cx.update(|cx| cx.set_global(update));
            }
            cx.background_executor().timer(DAY).await;
        }
    })
    .detach();
}

/// The status bar's item for an offered update, with a spinner while it installs.
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
            .when(u.installing, |d| d.child(ui::spinner(t.accent)))
            .child(label)
            .on_click(|_, _, cx| install(cx)),
    )
}

fn install(cx: &mut App) {
    let u = cx.global::<Update>();
    if u.installing {
        return;
    }
    if u.install == Install::Deb {
        return cx.open_url(RELEASES);
    }
    let (install, release) = (u.install.clone(), u.release.clone());
    cx.global_mut::<Update>().installing = true;
    let job = blocking::run(cx, move || install.apply(&download(&release, PUBLIC_KEY)?));
    cx.spawn(async move |cx| {
        let done = job.await;
        cx.update(|cx| match done {
            Ok(()) => cx.quit(),
            Err(e) => {
                cx.global_mut::<Update>().installing = false;
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

    const LATEST_JSON: &str = r#"{"version": "0.4.0", "platforms": {
        "macos-aarch64": {"url": "https://x/Detached-macos.zip", "signature": "mac"},
        "macos-x86_64": {"url": "https://x/Detached-macos.zip", "signature": "mac"},
        "windows-x86_64": {"url": "https://x/Detached-windows-x64-setup.exe", "signature": "win"},
        "linux-x86_64": {"url": "https://x/Detached-linux-x86_64.AppImage", "signature": "linux"}
    }}"#;

    #[test]
    fn latest_json_names_each_platforms_package() {
        let packages = [
            ("macos-aarch64", "Detached-macos.zip", "mac"),
            ("macos-x86_64", "Detached-macos.zip", "mac"),
            ("windows-x86_64", "Detached-windows-x64-setup.exe", "win"),
            ("linux-x86_64", "Detached-linux-x86_64.AppImage", "linux"),
        ];
        let json = LATEST_JSON.as_bytes();
        for (platform, file, signature) in packages {
            let want = Release {
                version: Version::new(0, 4, 0),
                url: format!("https://x/{file}"),
                signature: signature.into(),
            };
            assert_eq!(release(json, platform, "0.3.0").unwrap(), Some(want));
        }
        assert_eq!(release(json, "linux-aarch64", "0.3.0").unwrap(), None);
        assert!(release(&json[..40], "linux-x86_64", "0.3.0").is_err());
    }

    #[test]
    fn only_a_newer_version_is_offered() {
        let newer = |version: &str, current| {
            let json = LATEST_JSON.replace("0.4.0", version);
            release(json.as_bytes(), "linux-x86_64", current).map(|r| r.is_some())
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

    /// Serves `latest.json`, offering `version` for this platform as `/package`, on localhost.
    fn serve(version: &str, package: Vec<u8>, signature: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let platform = format!("{}-{}", env::consts::OS, env::consts::ARCH);
        let entry = serde_json::json!({"url": format!("{base}/package"), "signature": signature});
        let latest = serde_json::json!({"version": version, "platforms": {platform: entry}});
        let files = HashMap::from([
            ("/latest.json".to_string(), latest.to_string().into_bytes()),
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
        let base = serve("9.9.9", package.clone(), &sign(&package));
        let release = check(&base, "0.3.0").unwrap().expect("offered");
        assert_eq!(release.version, Version::new(9, 9, 9));
        assert_eq!(download(&release, &key).unwrap(), package);
    }

    #[test]
    fn the_running_version_is_not_offered() {
        let base = serve("0.3.0", vec![], "");
        assert_eq!(check(&base, "0.3.0").unwrap(), None);
    }

    #[test]
    fn a_missing_file_or_a_bad_signature_is_an_error() {
        let (key, sign) = keys();
        let base = serve("9.9.9", b"tampered".to_vec(), &sign(b"signed"));
        assert!(check(&format!("{base}/missing"), "0.3.0").is_err());
        let release = check(&base, "0.3.0").unwrap().unwrap();
        let err = download(&release, &key).unwrap_err().to_string();
        assert!(err.starts_with("signature: "), "{err}");
        let missing = Release {
            url: format!("{base}/missing"),
            ..release
        };
        assert!(download(&missing, &key).is_err());
    }

    #[test]
    fn a_package_over_10_mb_downloads() {
        let (key, sign) = keys();
        let package = vec![7; 11 << 20];
        let base = serve("9.9.9", package.clone(), &sign(&package));
        let release = check(&base, "0.3.0").unwrap().unwrap();
        assert_eq!(download(&release, &key).unwrap().len(), package.len());
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
