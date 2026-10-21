# Craft Apps Manager

<img src="assets/icon.png" width="80" alt="Craft Apps Manager icon">

Install, update, launch, and build Storytold Craft apps in one place. Manage multiple installations, follow progress in Activity, and add supported desktop apps from GitHub.

Available for **Windows, Linux, and macOS**. This is an independent community project, not an official Storytold or Adobe application.

**[Download](https://github.com/CryptoKey98/craft-apps-manager/releases/latest)** · [Documentation](docs/README.md) · [Changelog](CHANGELOG.md) · [Report a problem](https://github.com/CryptoKey98/craft-apps-manager/issues)

Version **1.0.0** includes signed manager updates, compatibility updates with fewer prompts, and Linux TAR.GZ and macOS DMG downloads. See the [changelog](CHANGELOG.md) for the full release notes.

![PhotoCraft's page in Craft Apps Manager](docs/images/main.png?v=0.8.0)

<details>
<summary>More screenshots</summary>

Screenshots show the Windows 0.8.0 interface with a demo library, captured at 2× resolution. Some controls have changed since then; the images do not show every current feature.

**Home** — launch installed apps and review updates.

![Home page](docs/images/overview.png?v=0.8.0)

**Install** — choose options for an individual app.

![Install confirmation](docs/images/install.png?v=0.8.0)

**Settings** — manage apps, checks, backups, builds, and folders.

![Settings](docs/images/settings.png?v=0.8.0)

**Update All selection** — choose which installed apps to update.

![Choose apps for Update all](docs/images/app-selection.png?v=0.8.0)

**Activity** — see progress and recent results.

![Activity panel](docs/images/activity.png?v=0.8.0)

**Builder** — build apps in a separate window.

![Craft Apps Builder](docs/images/builder.png?v=0.8.0)

</details>

## Get started

1. Download the package for your system from [Releases](https://github.com/CryptoKey98/craft-apps-manager/releases/latest).
2. Install it, or extract the entire portable archive into its own folder.
3. Open the manager, choose an app, and click **Get**. Review its version, format, architecture, and destination, then click **Install**.

| Your system | Standard installation | Portable download |
| --- | --- | --- |
| Windows 64-bit / 32-bit | x64 / x86 MSI | x64 / x86 ZIP |
| Ubuntu or Debian 64-bit / 32-bit | amd64 / i386 DEB | x64 / x86 ZIP or TAR.GZ |
| Fedora or RPM-based Linux 64-bit / 32-bit | x86_64 / i686 RPM | x64 / x86 ZIP or TAR.GZ |
| Arch Linux 64-bit (experimental) | x86_64 `.pkg.tar.zst` | x64 ZIP or TAR.GZ |
| macOS 11+, Intel or Apple silicon | Universal DMG | Universal ZIP |

**Windows:** run the MSI, or open `CraftApps-Manager.exe` from the extracted ZIP. Keep its bundled `workspace/tools/7zip` folder. Rust is not required to run the manager.

**Linux:** install the package matching your distribution, or extract the portable archive and run `craft-apps-manager`. See [Linux setup](docs/user-guide/linux.md) for commands and desktop requirements.

**macOS:** open the DMG and drag the app into Applications, or keep the extracted ZIP folder together with `portable.txt` beside the app. The manager is not notarized; macOS may require **System Settings → Privacy & Security → Open Anyway**. See [macOS setup](docs/user-guide/macos.md).

## What you can do

- **Manage apps:** install, launch, update, downgrade, move, or uninstall supported apps.
- **Keep multiple copies:** select which installation the app's actions apply to.
- **Review updates:** Update All works on selected installed apps. Automatic checks notify you; they do not automatically install updates.
- **Follow progress:** Manager and Builder share operation records, logs, and cancellation status in Activity.
- **Build from source:** set up tools, build supported Craft apps, and choose a completed build to launch.
- **Add GitHub apps:** review compatible downloads in **Settings → Apps → Custom apps**. Support is experimental and varies by package.

New Craft apps can appear after an app-list refresh without updating the manager. Adding or discovering an app does not install it.

See [Using the manager](docs/user-guide/using-the-manager.md) for detailed instructions and the [changelog](CHANGELOG.md) for release history.

## Your apps and data

Portable archives include `portable.txt`; new portable libraries use a `data` folder beside the manager. Move the whole extracted folder together. Existing external libraries stay external.

Installed versions use your operating system's user-data location. The **Data path at the bottom of the window** shows the active library. Back up that folder before making major changes.

See [Storage and portability](docs/user-guide/using-the-manager.md#storage-and-portability) for default paths, moving a library, and what does not move with it.

## Help and compatibility

Start with the [documentation index](docs/README.md), [Linux guide](docs/user-guide/linux.md), or [macOS guide](docs/user-guide/macos.md). If something fails, [report an issue](https://github.com/CryptoKey98/craft-apps-manager/issues) with your manager version/revision, operating system, installation type, steps, and exact error. Remove tokens and personal information from anything you share.

Available app downloads depend on each upstream project. Linux desktop integration varies by distribution and display server; Arch packages remain experimental. Native 32-bit operating systems and Linux ARM64 have not been verified. Platform guides describe known limitations.

## Build this manager

Install stable Rust and your platform's prerequisites, then run from the repository root:

```sh
cargo build --release --locked --bin craft-apps-manager
```

**This builds the default GitHub-only updater.** For the official service updater on Windows:

```powershell
python scripts/lib/updater_config.py build --config updater.production.toml --target-os windows
```

The script runs Cargo with the selected configuration and required feature. On Linux or macOS, use `python3` and change `--target-os` to `linux` or `macos`. Builds write to `target/release`; the most recent build determines which updater that executable uses.

See [Building and testing](docs/development/building.md) for prerequisites, executable locations, verification, and packaging. Maintainers should also read [Release preparation](docs/development/releases.md) and [Updater operations](docs/development/updater-operations.md).

## License

[MIT](LICENSE). Third-party components and artwork have their own notices in [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt) and the relevant asset folders.
