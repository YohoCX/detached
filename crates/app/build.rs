use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-env-changed=GHOSTTY_VT_DIR");
    // GPUI takes the window and taskbar icon from resource id 1.
    #[cfg(windows)]
    embed_resource::compile(
        "icons/icon.rc",
        embed_resource::ParamsIncludeDirs(["icons"]),
    )
    .manifest_optional()
    .unwrap();
    ghostty();
}

const VERSION: &str = "1.0.0";

/// Links libghostty-vt 1.0 statically: Apple's from the release's XCFramework, elsewhere built
/// with Zig 0.17 from the release's source and dependency tarballs. Every download is checked
/// against its SHA-256 from the release's signed SHA256SUMS. GHOSTTY_VT_DIR keeps the library
/// out of the target directory, so a clean build or CI's cache can reuse it.
fn ghostty() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let root = match std::env::var_os("GHOSTTY_VT_DIR") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ghostty-vt"),
    };
    let dir = root.join(VERSION);
    let (lib, file) = match os.as_str() {
        "macos" => (
            "ghostty-vt.xcframework/macos-arm64_x86_64",
            "libghostty-vt.a",
        ),
        "windows" => ("lib", "ghostty-vt-static.lib"),
        _ => ("lib", "libghostty-vt.a"),
    };
    if !dir.join(lib).join(file).exists() {
        // Built beside and moved into place whole, so a broken or concurrent build leaves no half.
        let tmp = root.join(format!("{VERSION}.{}", std::process::id()));
        let (src, out) = (tmp.join("src"), tmp.join("out"));
        // Failed builds' trees; a concurrent first build sharing GHOSTTY_VT_DIR would fail once.
        for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            if e.file_name()
                .to_string_lossy()
                .starts_with(&format!("{VERSION}."))
            {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
        std::fs::create_dir_all(&src).unwrap();
        if os == "macos" {
            let zip = format!("ghostty-vt-{VERSION}.xcframework.zip");
            fetch(
                &src,
                &zip,
                "2c32b7409b2b6c2ef6cfd606526de357762ad458bdc27134ad074e86506a8bc2",
            );
            run(Command::new("unzip")
                .arg("-q")
                .arg(src.join(zip))
                .arg("-d")
                .arg(&out));
        } else {
            for (name, sha) in [
                (
                    format!("libghostty-vt-{VERSION}.tar.gz"),
                    "4a6ef758be721923e1969e7b81b443dcc92d8dd3fb5a39600c03a8817a753676",
                ),
                (
                    format!("libghostty-vt-{VERSION}-deps.tar.gz"),
                    "4d5ea5e448e4c7ea8fb55ccbaa8668f2a97d44f438cee831e2b7ecd162d85c29",
                ),
            ] {
                fetch(&src, &name, sha);
                // Relative paths: Git's tar on Windows reads C: as a remote host.
                run(Command::new("tar").current_dir(&src).args(["-xzf", &name]));
            }
            // Zig 0.17 can crash on a global cache that is not new and empty.
            run(Command::new("zig")
                .current_dir(src.join(format!("libghostty-vt-{VERSION}")))
                .env("ZIG_GLOBAL_CACHE_DIR", tmp.join("zig-global"))
                .env("ZIG_LOCAL_CACHE_DIR", tmp.join("zig-local"))
                .args(["build", "--system"])
                .arg(format!("../libghostty-vt-{VERSION}-deps"))
                .args(["-Doptimize=ReleaseFast", "-Dcpu=baseline", "-Dstrip=true"])
                .arg("--prefix")
                .arg(&out));
        }
        // Another build may have won the race; its library is as good.
        if let Err(e) = std::fs::rename(&out, &dir) {
            assert!(dir.join(lib).join(file).exists(), "{}: {e}", dir.display());
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
    println!("cargo:rustc-link-search=native={}", dir.join(lib).display());
    if os == "windows" {
        println!("cargo:rustc-link-lib=static=ghostty-vt-static");
        // What the Zig standard library in it calls.
        println!("cargo:rustc-link-lib=ntdll");
        println!("cargo:rustc-link-lib=kernel32");
    } else {
        println!("cargo:rustc-link-lib=static=ghostty-vt");
    }
}

/// Downloads `name` from the release into `dir`, checked against its pinned SHA-256.
fn fetch(dir: &Path, name: &str, sha256: &str) {
    use sha2::Digest;
    let path = dir.join(name);
    run(Command::new("curl")
        .current_dir(dir)
        .args(["-fsSL", "--retry", "3", "-o", name])
        .arg(format!(
            "https://release.files.ghostty.org/libghostty-vt/{VERSION}/{name}"
        )));
    let hash = sha2::Sha256::digest(std::fs::read(&path).unwrap());
    assert_eq!(
        format!("{hash:x}"),
        sha256,
        "{name} does not match its pinned hash"
    );
}

fn run(c: &mut Command) {
    let status = c.status();
    let hint = match c.get_program() == "zig" {
        true => {
            "\nZig 0.17 builds libghostty-vt here: put zig on PATH, or a built one in GHOSTTY_VT_DIR/<version>"
        }
        false => "",
    };
    assert!(
        status.as_ref().is_ok_and(|s| s.success()),
        "{c:?}: {status:?}{hint}"
    );
}
