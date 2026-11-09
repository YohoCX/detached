fn main() {
    println!("cargo:rerun-if-changed=icons");
    // GPUI takes the window and taskbar icon from resource id 1.
    #[cfg(windows)]
    embed_resource::compile(
        "icons/icon.rc",
        embed_resource::ParamsIncludeDirs(["icons"]),
    )
    .manifest_optional()
    .unwrap();
    #[cfg(feature = "ghostty")]
    ghostty();
}

/// Links libghostty-vt from its release's XCFramework, checked against a pinned SHA-256.
#[cfg(feature = "ghostty")]
fn ghostty() {
    use sha2::Digest;
    use std::process::Command;
    const URL: &str =
        "https://release.files.ghostty.org/libghostty-vt/1.0.0/ghostty-vt-1.0.0.xcframework.zip";
    const SHA256: &str = "2c32b7409b2b6c2ef6cfd606526de357762ad458bdc27134ad074e86506a8bc2";
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    assert_eq!(os, "macos", "libghostty-vt ships prebuilt for Apple only");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let lib = out.join("ghostty-vt.xcframework/macos-arm64_x86_64");
    if !lib.join("libghostty-vt.a").exists() {
        let zip = out.join("ghostty-vt.zip");
        let run = |c: &mut Command| assert!(c.status().is_ok_and(|s| s.success()), "{c:?}");
        run(Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&zip)
            .arg(URL));
        let hash = sha2::Sha256::digest(std::fs::read(&zip).unwrap());
        assert_eq!(
            format!("{hash:x}"),
            SHA256,
            "{URL} does not match its pinned hash"
        );
        run(Command::new("unzip")
            .arg("-qo")
            .arg(&zip)
            .arg("-d")
            .arg(&out));
    }
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
}
