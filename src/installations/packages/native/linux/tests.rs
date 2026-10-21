use super::*;
#[cfg(target_os = "linux")]
struct PackageFixture(PathBuf);
#[cfg(target_os = "linux")]
impl PackageFixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("craft-linux-fixture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn write(&self, path: &str, content: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    fn payload(&self) {
        self.write("usr/bin/different-name", "#!/bin/sh\nexit 0\n");
        self.write("usr/bin/second-app", "#!/bin/sh\nexit 0\n");
        self.write(
            "usr/share/applications/app.desktop",
            "[Desktop Entry]\nType=Application\nName=Fixture\nExec=different-name %U\n",
        );
    }
}
#[cfg(target_os = "linux")]
impl Drop for PackageFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[cfg(target_os = "linux")]
#[test]
fn real_deb_metadata_selects_package_owned_desktop_executable() {
    if Command::new("dpkg-deb").arg("--version").output().is_err() {
        eprintln!("DEB fixture skipped: dpkg-deb is unavailable on this distribution");
        return;
    }
    let fixture = PackageFixture::new();
    fixture.payload();
    fixture.write("DEBIAN/control", "Package: unrelated-repo-package\nVersion: 1.0\nArchitecture: all\nMaintainer: Fixture <fixture@example.invalid>\nDescription: Fixture only\n");
    let output = PackageFixture::new();
    let archive = output.0.join("fixture.deb");
    assert!(Command::new("dpkg-deb")
        .arg("--build")
        .arg(&fixture.0)
        .arg(&archive)
        .status()
        .unwrap()
        .success());
    assert_eq!(
        package_name(&archive, PackageKind::Debian).unwrap(),
        "unrelated-repo-package"
    );
    assert_eq!(
        package_executable(
            &archive,
            PackageKind::Debian,
            "photocraft",
            "unrelated-repo-package",
            None
        )
        .unwrap(),
        "/usr/bin/different-name"
    );
}
#[cfg(target_os = "linux")]
#[test]
fn real_arch_metadata_rejects_unowned_exec_and_reuses_learned_choice() {
    let fixture = PackageFixture::new();
    fixture.payload();
    fixture.write(
        ".PKGINFO",
        "pkgname = unrelated-repo-package\npkgver = 1.0-1\narch = x86_64\n",
    );
    let archive = fixture.0.join("fixture.pkg.tar");
    let build = || {
        assert!(Command::new("bsdtar")
            .arg("-cf")
            .arg(&archive)
            .arg("-C")
            .arg(&fixture.0)
            .args([".PKGINFO", "usr"])
            .status()
            .unwrap()
            .success());
    };
    build();
    assert_eq!(arch_metadata(&archive).unwrap().0, "unrelated-repo-package");
    assert_eq!(
        package_executable(
            &archive,
            PackageKind::Arch,
            "photocraft",
            "unrelated-repo-package",
            None
        )
        .unwrap(),
        "/usr/bin/different-name"
    );
    fixture.write(
        "usr/share/applications/second.desktop",
        "[Desktop Entry]\nType=Application\nExec=second-app\n",
    );
    build();
    assert!(package_executable(
        &archive,
        PackageKind::Arch,
        "photocraft",
        "unrelated-repo-package",
        None
    )
    .is_err());
    crate::app_identity::remember(
        &fixture.0,
        "photocraft",
        crate::app_identity::NativeIdentity {
            launch: None,
            family: String::new(),
            platform: "linux".into(),
            package: "unrelated-repo-package".into(),
            executable: "/usr/bin/second-app".into(),
        },
    )
    .unwrap();
    assert_eq!(
        package_executable(
            &archive,
            PackageKind::Arch,
            "photocraft",
            "unrelated-repo-package",
            None
        )
        .unwrap(),
        "/usr/bin/second-app"
    );
    fixture.write(
        "usr/share/applications/app.desktop",
        "[Desktop Entry]\nType=Application\nExec=/opt/not-owned\n",
    );
    std::fs::remove_file(fixture.0.join("usr/share/applications/second.desktop")).unwrap();
    build();
    assert!(package_executable(
        &archive,
        PackageKind::Arch,
        "photocraft",
        "unrelated-repo-package",
        None
    )
    .is_err());
    crate::app_identity::load(&fixture.0.join("empty"));
}
#[test]
fn failed_package_transaction_preserves_previous_launch_identity() {
    let root =
        std::env::temp_dir().join(format!("craft-package-identity-{}", uuid::Uuid::new_v4()));
    let old = crate::app_identity::NativeIdentity {
        platform: "linux".into(),
        package: "verified-package".into(),
        executable: "/usr/bin/old-app".into(),
        ..Default::default()
    };
    crate::app_identity::remember(&root, "photocraft", old.clone()).unwrap();
    let pending = crate::app_identity::NativeIdentity {
        executable: "/opt/new-app".into(),
        ..old.clone()
    };
    assert!(
        with_package_identity("photocraft", Some(&pending), || Err(anyhow::anyhow!(
            "authorization cancelled"
        )))
        .is_err()
    );
    crate::app_identity::load(&root);
    assert_eq!(crate::app_identity::native("photocraft"), vec![old]);
    crate::app_identity::load(&root.join("empty"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_launch_keeps_arguments_and_environment_without_shell_evaluation() {
    let text = "[Desktop Entry]\nExec=env APP_MODE=desktop \"APP_LABEL=two words\" /opt/app --safe \"two words\" %% %U";
    let (executable, launch) = desktop_launch(text).unwrap();
    assert_eq!(executable, "/opt/app");
    assert_eq!(launch.arguments, ["--safe", "two words", "%"]);
    assert_eq!(launch.environment["APP_LABEL"], "two words");
    let mut command = Command::new(&executable);
    launch.apply(&mut command);
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["--safe", "two words", "%"]
    );
    assert!(command
        .get_envs()
        .any(|(name, value)| name == "APP_MODE" && value == Some(std::ffi::OsStr::new("desktop"))));
    for exec in [
        "env -S 'app --safe'",
        "sh -c app",
        "env 9INVALID=x /opt/app",
        "env BAD=x BAD=y /opt/app",
        "/opt/app %k",
        "/opt/app $HOME",
        "\"unterminated",
    ] {
        assert!(
            desktop_launch(&format!("[Desktop Entry]\nExec={exec}")).is_none(),
            "{exec}"
        );
    }
    let files = vec!["/opt/app".into()];
    let desktops = vec![
        "[Desktop Entry]\nExec=env MODE=one /opt/app".into(),
        "[Desktop Entry]\nExec=env MODE=two /opt/app".into(),
    ];
    assert_eq!(desktop_launch_candidates(&files, &desktops).len(), 2);
    assert!(desktop_launch_candidates(&[], &desktops).is_empty());
}

#[test]
fn desktop_env_launcher_resolves_the_application_instead_of_env() {
    assert_eq!(
        desktop_exec(
            "[Desktop Entry]\nExec=/usr/bin/env APP_MODE=desktop \"/opt/App Name/app\" --safe %U"
        ),
        Some("/opt/App Name/app".into())
    );
    assert!(desktop_exec("[Desktop Entry]\nExec=sh -c \"/opt/app; touch /tmp/no\"").is_none());
}

#[test]
fn desktop_identity_requires_exact_package_owned_command() {
    let files = vec!["/usr/bin/different-name".into(), "/usr/bin/helper".into()];
    assert_eq!(
        desktop_candidates(&files, &["[Desktop Entry]\nExec=different-name %U".into()]),
        vec!["/usr/bin/different-name"]
    );
    assert!(desktop_candidates(&files, &["[Desktop Entry]\nExec=/opt/unowned".into()]).is_empty());
    assert!(desktop_candidates(&files, &["[Desktop Action Help]\nExec=helper".into()]).is_empty());
    assert_eq!(
        desktop_exec("[Desktop Entry]\nExec=\"/opt/App Name/app\" %F"),
        Some("/opt/App Name/app".into())
    );
    assert!(desktop_exec("[Desktop Entry]\nType=Link\nExec=helper").is_none());
    assert!(desktop_exec("[Desktop Entry]\nExec=helper\nTerminal=true").is_none());
    assert!(!safe_linux_path("/usr/bin/../app"));
    assert!(!safe_package("--remove"));
    assert!(!safe_package("app\nother"));
    assert!(safe_package("different-name+desktop"));
}
#[test]
fn explicit_downgrades_use_native_package_manager_support() {
    assert_eq!(
        installation_arguments(PackageKind::Debian, true),
        ["apt-get", "install", "-y", "--allow-downgrades"]
    );
    assert_eq!(
        installation_arguments(PackageKind::Rpm, true),
        ["dnf", "downgrade", "-y"]
    );
    assert_eq!(
        installation_arguments(PackageKind::Rpm, false),
        ["dnf", "install", "-y"]
    );
    assert_eq!(
        installation_arguments(PackageKind::Arch, true),
        ["pacman", "-U", "--noconfirm"]
    );
}
#[test]
fn arch_metadata_requires_unique_identity_fields() {
    let info = "pkgname = pdfcraft\npkgver = 1:0.4.1-2\narch = x86_64\ndepend = glibc\n";
    let (name, version, arch) = parse_arch_metadata(info).unwrap();
    assert_eq!(name, "pdfcraft");
    assert_eq!(upstream_version(&version), "0.4.1");
    assert_eq!(arch, "x86_64");
    assert!(parse_arch_metadata("pkgname = pdfcraft\narch = x86_64").is_err());
    assert!(parse_arch_metadata(&format!("{info}pkgname = other\n")).is_err());
}
#[test]
fn distro_identity_determines_package_backend() {
    assert_eq!(
        kind_from_os_release("ID=fedora\n").unwrap(),
        PackageKind::Rpm
    );
    assert_eq!(
        kind_from_os_release("ID=rocky\nID_LIKE=\"rhel centos fedora\"\n").unwrap(),
        PackageKind::Rpm
    );
    assert_eq!(
        kind_from_os_release("ID=ubuntu\nID_LIKE=debian\n").unwrap(),
        PackageKind::Debian
    );
    assert_eq!(
        kind_from_os_release("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n").unwrap(),
        PackageKind::Debian
    );
    assert_eq!(
        kind_from_os_release("ID=arch\n").unwrap(),
        PackageKind::Arch
    );
    assert_eq!(
        kind_from_os_release("ID=manjaro\nID_LIKE=arch\n").unwrap(),
        PackageKind::Arch
    );
    assert!(kind_from_os_release("ID=notfedora\n").is_err());
}
