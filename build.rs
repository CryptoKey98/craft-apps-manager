fn main() {
    if std::env::var("TARGET").is_ok_and(|target| target.ends_with("windows-msvc")) {
        // Declare normal user privileges so Windows does not mistake the
        // 32-bit updater (or its test binaries) for an installer by name.
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTUAC:level='asInvoker' uiAccess='false'");
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let timestamp = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("System clock before Unix epoch")
                .as_secs()
        });
    println!("cargo:rustc-env=CRAFT_BUILD_TIMESTAMP={timestamp}");
    println!(
        "cargo:rustc-env=CRAFT_BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap()
    );
    println!(
        "cargo:rustc-env=CRAFT_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
