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
}
