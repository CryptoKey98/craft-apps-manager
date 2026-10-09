fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .set("ProductName", "Craft Apps Manager")
            .set("FileDescription", "Craft Apps Manager")
            .set("OriginalFilename", "CraftApps-Manager.exe")
            .compile()
            .expect("Failed to embed the application icon");
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.png");
    if std::env::var("TARGET").is_ok_and(|target| target.ends_with("windows-msvc")) {
        // Declare normal user privileges so Windows does not mistake the
        // 32-bit manager (or its test binaries) for an installer by name.
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTUAC:level='asInvoker' uiAccess='false'");
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    // GitHub repository (owner/name) that manager self-updates come from.
    // Forks can build against their own releases.
    println!("cargo:rerun-if-env-changed=CRAFT_MANAGER_REPOSITORY");
    let repository = std::env::var("CRAFT_MANAGER_REPOSITORY")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "CryptoKey98/craft-apps-manager".into());
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    };
    match repository.split_once('/') {
        Some((owner, name)) if valid(owner) && valid(name) => {}
        _ => panic!("CRAFT_MANAGER_REPOSITORY must be owner/name"),
    }
    println!("cargo:rustc-env=CRAFT_MANAGER_REPOSITORY={repository}");
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
