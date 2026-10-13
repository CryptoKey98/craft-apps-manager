# Craft Apps Manager

<img src="assets/icon.png" width="80" alt="Craft Apps Manager icon">

Install, update, launch, and build the Craft apps from Storytold in one place. Choose installation options for each app, keep multiple copies, and follow your downloads and builds in Activity.

Version **0.8.2** includes Home installation switching, reusable portable-copy letters, improved app discovery, and safer folder and build-cache cleanup. See the [changelog](CHANGELOG.md) for all changes.

Available for **Windows, Linux, and macOS**. This is an independent community project, not an official Storytold or Adobe application.

**[Download the latest release](https://github.com/CryptoKey98/craft-apps-manager/releases/latest)** · [What's changed](CHANGELOG.md) · [Report a problem](https://github.com/CryptoKey98/craft-apps-manager/issues)

![PhotoCraft's page in Craft Apps Manager](docs/images/main.png?v=0.8.0)

<details>
<summary>More screenshots</summary>

Screenshots show the Windows 0.8.0 interface with a demo library, captured at 2× resolution. Some controls have changed since then; these images do not show every 0.8.2 feature.

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

1. Download the package for your operating system from [Releases](https://github.com/CryptoKey98/craft-apps-manager/releases/latest).
2. Install the package, or extract the portable ZIP into its own folder.
3. Open Craft Apps Manager, choose an app, and click **Get**. Review its version, format, architecture, and destination before clicking **Install**.

| Your system | Installer | Portable download |
| --- | --- | --- |
| Windows 64-bit / 32-bit | x64 / x86 MSI | x64 / x86 ZIP |
| Ubuntu or Debian 64-bit / 32-bit | amd64 / i386 DEB | x64 / x86 ZIP |
| Fedora or RPM-based Linux 64-bit / 32-bit | x86_64 / i686 RPM | x64 / x86 ZIP |
| Arch Linux 64-bit (experimental) | x86_64 `.pkg.tar.zst` | x64 ZIP |
| macOS 11+, Intel or Apple silicon | — | Universal ZIP |

**Windows:** run the MSI, or open `CraftApps-Manager.exe` in the extracted ZIP. Keep the ZIP's bundled `workspace/tools/7zip` folder. Rust is needed only for building apps, not for running the manager.

**Linux:** install the package matching your distribution, or extract the ZIP and run `craft-apps-manager`. Native package operations may ask for your administrator password. See [Linux setup](docs/linux.md) for commands and dependencies.

**macOS:** keep the extracted portable folder together, including `portable.txt` beside the `.app` bundle. The manager is not Apple-notarized; if macOS blocks its first launch, use **System Settings → Privacy & Security → Open Anyway**. See [macOS setup](docs/macos.md) for the full instructions and the optional Terminal installer.

## Everyday use

### Install and manage apps

- **Install options belong to each app.** Choose a version, release format, and architecture in its installation window. Only compatible combinations are offered; there are no global format or architecture controls in Settings.
- **Open, update, or uninstall** from the app page. Launch options let you select an executable and add arguments.
- **Keep multiple installations.** After the first install, use the **Installation** dropdown to select the copy you want to manage. **Add another installation…** opens the usual install window. Portable copies need separate folders; one native installer copy is supported. Apps may share their own user settings between copies.
- **Identify portable copies by letter.** The installation selectors and desktop shortcuts use matching letters. Removing B leaves A and C unchanged; the next new copy uses B. Home's compact installation switcher shares the app page's selection.
- **Move a copy** with **Move…** where supported: managed portable apps and supported Windows MSI installations. Linux native packages and macOS installer copies keep their system-managed locations.
- **Hide apps** from the sidebar or Home context menu. Restore them in **Settings → Apps → App visibility**. Hiding does not uninstall an app or change its update selection.

Updates and version changes use the selected installation's verified format and architecture. Unknown installation details are reported rather than guessed. Changing an existing copy's format or architecture requires removing that copy first, or adding a separate supported installation.

### Updates and Activity

**Check for updates** checks one app. **Update All** on Home updates the selected installed apps after a review; it never installs missing apps. Newly installed apps are selected automatically. With multiple copies, updates target the selected copy of each app.

In **Settings → Updates**, configure startup checks, periodic checks while the manager is running, or hourly scheduled checks. Checks do not automatically download or install updates. The manager's own update check is separate and displays **Update available** in the header when a matching release is available.

**Activity**, beside Settings, shows progress, cancellation, and results. Manager and Builder share operation records. Closing a dialog does not cancel its operation; use its Cancel button. Native package transactions may need to finish before cancellation can be confirmed.

To hide the Activity button's notification badge, enable **Hide activity badge** in **Settings → General**. Unread activity highlights still appear inside the panel.

**Approve administrator access once per batch** is enabled by default for eligible Windows and Linux installer batches. Linux uses PolicyKit with apt, dnf, or pacman. Individual installers can still show their own prompts. macOS uses its existing authorization workflow.

**What's New** appears on first launch and after a version change. Reopen it with the info icon beside the manager version.

### Installer cleanup

Starting with **0.8.1**, downloaded setup files are automatically removed after a successful, verified installation or update. Cleanup appears in the operation's progress and Activity details.

Older setup files left by previous versions are not automatically swept up. Recovery packages or files required for installer maintenance may be retained. Failed or cancelled installations do not run successful-install cleanup; deletion problems are reported without marking a verified installation as failed.

Version 0.8.2 also removes an app's empty setup-download folder after cleanup. Additional portable library copies use flat folders such as `filmcraft-2`; uninstalling removes empty legacy wrapper folders while preserving user-selected parents and folders containing other files.

### Run in the background

Enable **Keep running in system tray when closed** in General settings to hide the window while checks continue. The tray offers **Open**, **Check for app updates**, and **Exit**.

This needs a usable desktop tray. On Linux, some GNOME desktops require a compatible tray extension. If there is no tray host, closing quits normally so the manager cannot become inaccessible.

## Custom GitHub apps

Paste a repository link into **Settings → Apps → Custom apps**. The setup dialog lets you review the detected download, package type, and architecture, and choose a download when several are possible.

Adding a repository adds it to the list; it does not install the app. Removing it from the list does not uninstall it. A repository needs a compatible downloadable release for your system.

**Custom-app support is experimental.** Asset naming and installer registration vary, so some apps may not install, be detected, or uninstall reliably. Third-party apps are not included by default.

An optional GitHub token can be saved under **Settings → Updates** to raise the API limit. It is stored in the operating system's credential store. Environment variables take priority over the saved token: `CRAFT_GITHUB_TOKEN`, then `GITHUB_TOKEN`. Authentication is sent only to `api.github.com`; don't post tokens in issues or logs.

## Craft app discovery

The manager includes the main Craft apps, including DesignCraft, EffectCraft, FilmCraft, LightCraft, PhotoCraft, PDFCraft, VectorCraft, WordCraft, GridCraft, DeckCraft, CADCraft, and SoundCraft. ArtCraft has release support; ArtCraft X has source-build support. Available downloads depend on what each app publishes for your platform.

New Storytold Craft apps can be discovered without updating the manager. Use **Settings → Updates → App list → Refresh**. Startup discovery is enabled by default and uses a shared six-hour cache. Discovery adds apps to the list without installing them or selecting them for updates.

Older PrintCraft files remain compatible with PDFCraft's library identity. App discovery and ArtCraft support were contributed by [carpaaaaa](https://github.com/carpaaaaa).

## Portable folders and saved data

Portable ZIPs include `portable.txt` and use portable storage automatically. A new portable library lives in a `data` folder beside the manager. Existing external libraries are preserved during migration.

To move a portable manager, finish active operations, close **both Manager and Builder**, and move the **whole extracted folder**. Keep `portable.txt`, `portable-state.json`, and bundled resources with it. Managed paths inside that folder follow the move; external locations remain unchanged.

Native installations, system-stored tokens, and desktop shortcuts do not move with the folder. Shortcuts may need recreation, and individual apps may store preferences elsewhere. Moving between operating systems does not convert apps or builds. Don't run an older manager against migrated portable metadata.

| Manager installation | Default library location |
| --- | --- |
| Portable ZIP | `data` inside the portable folder; existing external libraries are retained |
| Windows MSI | `%LOCALAPPDATA%\Craft Apps Manager` |
| Linux system package | `$XDG_DATA_HOME/craft-apps-manager`, or `~/.local/share/craft-apps-manager` |
| macOS installation using system storage | `~/Library/Application Support/Craft Apps Manager` |

The **Data path at the bottom of the window** identifies your active library. **Settings → Folders** shows its locations and portable-storage information.

<details>
<summary>What's inside the library?</summary>

```text
releases/           Portable apps and downloaded setup files
sources/            Source archives and their index
builds/             Finished builds, grouped by app
logs/               Update and build logs
backups/releases/   Portable app backups
backups/sources/    Source backups
workspace/          Build tools, extracted sources, and build cache
runtime/            Staging, operation records, and migration recovery copies
```

`settings.json` records installations; `manager-settings.json` contains manager preferences. Folder changes that require a restart are shown in Settings.

</details>

## Build apps and manage backups

Use an app's tools to **Set up build tools**, download its source, and **Build** or **Rebuild**. Open the separate Builder through **Settings → Builds → Open builder window**. Build Options lets you choose which completed build the Launch button uses; that choice is shared between Manager and Builder.

Builds use upstream source and lockfiles. Upstream compilation failures are shown in the log. Required tools depend on the app and platform; Windows Rust builds need Microsoft's C++ build tools, and ArtCraft X also needs frontend tools. Failed or cancelled builds keep their cache for a retry.

**Build Options** can delete temporary cache from interrupted builds, even before a successful build exists. Cleanup is unavailable during a build and preserves completed builds and source ZIPs. The next build recreates its cache.

Optional portable-app and source backups are configured in Settings. Use **Backups → Manage…** on an app page to restore or delete a selected backup. Native installer applications are not backed up by the manager. There is no universal automatic rollback for every installer.

Uninstalling can optionally remove listed app profile folders. This is off by default, and some installations share profiles. Review the confirmation before deleting profile data.

## Compatibility and help

Windows x64 and x86 packages have been tested on 64-bit Windows. Linux x64 desktop tests cover Ubuntu 26.04, Fedora 44, and Arch; Arch remains experimental. Linux x86 launch tests used multilib on 64-bit VMs. Native 32-bit systems and Linux ARM64 have not been verified, and macOS desktop testing remains limited.

Published Linux ZIP, DEB, and RPM builds use Ubuntu 22.04 as their baseline; Arch packages are built separately. A build made locally on a newer distribution may require newer system libraries.

For a problem report, include the manager version, operating system, affected app, installation type, and relevant **Activity details** or logs. For detection issues, include `settings.json` from the Data folder. Redact personal paths and never share tokens or passwords.

- [Linux installation and builds](docs/linux.md)
- [macOS installation and builds](docs/macos.md)
- [Windows packaging](docs/windows-packaging.md)
- [Per-app installation behavior](docs/per-app-installation.md)
- [Operations and cancellation](docs/operations.md)
- [Windows batch approval](docs/windows-batch-admin.md) · [Linux batch approval](docs/linux-batch-admin.md)

<details>
<summary>Upgrading from the old Craft Apps Updater</summary>

The app was renamed to Craft Apps Manager in 0.3.2. Very old versions may reject the renamed repository or package, so download Manager manually. Close the old Updater and preserve your existing library when switching. Disable old scheduled tasks before configuring Manager's scheduled checks.

</details>

## Build this manager

The source is shared across Windows, Linux, and macOS. Install stable Rust and your platform's native prerequisites; see the platform guides above.

```sh
cargo build --release --locked --bin craft-apps-manager
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

On Windows, install Microsoft's **Desktop development with C++** workload. Platform-specific modules live in `src/platform`, `src/installers`, `src/scheduler`, and `src/tools`. Build output is excluded from the source repository. Some integration checks need local tools; see the test instructions in the platform docs.

For packaging and version preparation, see [Release preparation](docs/releases.md).

## License

MIT. Bundled 7-Zip files have separate license notices. Craft apps and their branding belong to their respective authors.
