fn main() {
    println!("cargo:rerun-if-env-changed=CRAFT_UPDATER_FIXTURE_VERSION");
    let fixture_version =
        std::env::var("CRAFT_UPDATER_FIXTURE_VERSION").unwrap_or_else(|_| "1.0.0".into());
    assert!(
        fixture_version
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.'),
        "Invalid fixture version"
    );
    println!("cargo:rustc-env=CRAFT_UPDATER_FIXTURE_VERSION={fixture_version}");
    let variables = [
        "CRAFT_UPDATER_CONFIG",
        "CRAFT_UPDATE_SOURCE",
        "CRAFT_MANAGER_REPOSITORY",
        "CRAFT_UPDATE_CATALOG_URL",
        "CRAFT_UPDATE_CATALOG_MIRROR",
        "CRAFT_UPDATE_CHECK_URL",
        "CRAFT_BUILD_REVISION",
    ];
    for name in variables {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let project =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("project root"));
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("target OS");
    let config = craft_updater_config::resolve(
        &project,
        None,
        &std::env::vars()
            .filter(|(k, _)| variables.contains(&k.as_str()))
            .collect(),
        &target_os,
        &Default::default(),
    )
    .unwrap_or_else(|e| panic!("Invalid updater configuration: {e:#}"));
    config
        .require_build_support(std::env::var_os("CARGO_FEATURE_SERVICE_UPDATER").is_some())
        .unwrap_or_else(|e| panic!("{e:#}"));
    let identity = config
        .build_identity(
            &std::env::var("CARGO_PKG_VERSION").expect("package version"),
            &std::env::var("TARGET").expect("build target"),
        )
        .to_string()
        .into_bytes();
    // A dedicated referenced section lets packaging verify every Mach-O slice
    // without executing code for the other CPU architecture.
    let generated = format!(
        "#[used]\n#[cfg_attr(target_os = \"macos\", link_section = \"__TEXT,__craft_update\")]\nstatic CRAFT_UPDATER_BUILD_IDENTITY: [u8; {}] = {:?};\n",
        identity.len(), identity,
    );
    std::fs::write(
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("build output"))
            .join("updater-build-identity.rs"),
        generated,
    )
    .expect("write embedded updater identity");
    println!("cargo:rerun-if-changed={}", config.config_path.display());
    println!(
        "cargo:rerun-if-changed={}",
        config.public_key_path.display()
    );
    for (name, value) in [
        ("CRAFT_UPDATE_SOURCE", config.config.source.clone()),
        ("CRAFT_MANAGER_REPOSITORY", config.config.repository.clone()),
        (
            "CRAFT_UPDATE_CATALOG_URL",
            config.config.service.catalog_url.clone(),
        ),
        (
            "CRAFT_UPDATE_CATALOG_MIRROR",
            config.config.service.mirror_url.clone(),
        ),
        (
            "CRAFT_UPDATE_CHECK_URL",
            config.config.service.check_url.clone(),
        ),
        ("CRAFT_BUILD_REVISION", config.revision.to_string()),
        ("CRAFT_UPDATE_CHANNEL", config.config.channel.clone()),
        ("CRAFT_UPDATE_PUBLIC_KEY_HEX", config.public_key_hex.clone()),
        (
            "CRAFT_UPDATE_PUBLIC_KEY_FINGERPRINT",
            config.public_key_fingerprint.clone(),
        ),
        (
            "CRAFT_UPDATE_CONNECT_TIMEOUT",
            config.config.network.connect_timeout_seconds.to_string(),
        ),
        (
            "CRAFT_UPDATE_METADATA_TIMEOUT",
            config.config.network.metadata_timeout_seconds.to_string(),
        ),
        (
            "CRAFT_UPDATE_DOWNLOAD_TIMEOUT",
            config.config.network.download_timeout_seconds.to_string(),
        ),
    ] {
        println!("cargo:rustc-env={name}={value}");
    }
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
