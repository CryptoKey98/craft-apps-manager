# Craft Apps Manager

<img src="assets/icon.png" width="80" alt="Craft Apps Manager icon">

A desktop app for downloading, updating, and building the Craft apps from Storytold. I made this to keep the apps, source downloads, and builds in one place without having to manage every release by hand.

The interface is written in Rust and shares one codebase across Windows, Linux and macOS. Version 0.7.0 adds a redesigned Home screen, release version selection, installation and build dialogs, app visibility controls and optional close-to-tray behavior. Pick an app from the sidebar to install, open or update it, with launch options, backups, sources and builds alongside. This is an independent project, not an official Storytold or Adobe app.

App discovery and ArtCraft support were contributed by [carpaaaaa](https://github.com/carpaaaaa). Use Settings → Updates → App list → Refresh to find new Storytold Craft apps. Discovery on startup is enabled by default and uses a shared six-hour cache. Including newly discovered apps in update selections is a separate opt-in setting. Discovery never installs apps.

[Download the latest release](https://github.com/CryptoKey98/craft-apps-manager/releases/latest)

![PhotoCraft's page in Craft Apps Manager](docs/images/main.png?v=0.7.0)

Screenshots show the Windows 0.7.0 release with a separate demo library.

<details>
<summary>More screenshots</summary>

**Home** — update chosen apps, launch installed apps, arrange app tiles, and follow the activity log.

![Home page](docs/images/overview.png?v=0.7.0)

**Install** — Install, Update and Get ask for confirmation before downloading.

![Install confirmation](docs/images/install.png?v=0.7.0)

**Settings** — one scrolling page for release format, updates, backups, builds and folders.

![Settings](docs/images/settings.png?v=0.7.0)

**App selection** — choose which installed apps Update All updates. Uninstalled apps cannot be selected.

![Choose apps for Update all](docs/images/app-selection.png?v=0.7.0)

**Builder** — the separate builder window builds any upstream source, including ArtCraft X.

![Craft Apps Builder](docs/images/builder.png?v=0.7.0)

</details>

## Getting started

Download the package for your system from the [Releases page](https://github.com/CryptoKey98/craft-apps-manager/releases/latest). Version **0.7.0** includes these formats:

| System | Installation | Portable |
| --- | --- | --- |
| Windows 64-bit | x64 MSI | x64 ZIP |
| Windows 32-bit | x86 MSI | x86 ZIP |
| Ubuntu / Debian 64-bit | amd64 DEB | x64 ZIP |
| Ubuntu / Debian 32-bit | i386 DEB | x86 ZIP |
| Fedora / RPM-based Linux 64-bit | x86_64 RPM | x64 ZIP |
| Fedora / RPM-based Linux 32-bit | i686 RPM | x86 ZIP |
| Arch Linux 64-bit (experimental) | x86_64 `.pkg.tar.zst` | x64 ZIP |
| macOS 11 or newer, Apple silicon and Intel | — | macos-universal ZIP |

On Linux, choose the native package for your distribution, or extract the ZIP and run `craft-apps-manager`. DEB files are for Ubuntu/Debian, RPM files are for Fedora and compatible systems, and `.pkg.tar.zst` files are for Arch. See [Linux setup and packaging](docs/linux.md) for installation and source-build instructions.

On macOS you can also install or update the manager from Terminal. The script downloads the latest release, checks it against `SHA256SUMS.txt` and copies the app into `/Applications` (or `~/Applications`); a copy installed this way opens without the first-launch steps below:

```text
curl -fsSL https://raw.githubusercontent.com/CryptoKey98/craft-apps-manager/main/scripts/install-macos.sh | bash
```

For Windows, run the MSI or extract the ZIP and open `CraftApps-Manager.exe`. Keep the portable package's `workspace/tools/7zip` folder with the app. Rust and a separate 7-Zip installation are not needed to run Manager.

The Windows MSI stores settings and the app library in `%LOCALAPPDATA%\Craft Apps Manager`, separate from program files. Portable builds use their own folder. Manager updates choose the matching MSI or ZIP, and uninstalling the manager keeps its library and settings. See [Windows packaging](docs/windows-packaging.md).

Linux DEB, RPM and portable release builds use Ubuntu 22.04 as their build baseline; Arch packages are built separately on Arch. Desktop testing has used Ubuntu 26.04 and Fedora 44. Older distributions still need testing, and Arch support remains experimental. x86 launch tests used 64-bit VMs with 32-bit libraries; native 32-bit systems and Linux ARM64 are not verified. Both Windows packages were tested on 64-bit Windows; native 32-bit Windows testing remains pending.

For macOS, extract the ZIP and move `Craft Apps Manager.app` to your Applications folder. The manager is not notarized by Apple yet, so the first launch shows "Apple could not verify “Craft Apps Manager.app” is free of malware". To open it:

1. Click **Done** in that dialog. Don't choose **Move to Trash**.
2. Open **System Settings → Privacy & Security** and scroll down to the message about Craft Apps Manager.app.
3. Click **Open Anyway** and confirm with your password or Touch ID.

You only need to do this once. Right-clicking the app and choosing **Open** no longer skips this check on macOS 15 and newer. The Craft apps themselves are notarized by their authors and open normally. Updates installed by the manager's own update check don't need this step again. On macOS, installer mode copies each Craft app into `/Applications` and also detects apps you installed yourself. See [macOS](docs/macos.md).

Changes are listed in [CHANGELOG.md](CHANGELOG.md). The source is shared across Windows, Linux and macOS.

## Upgrading from Craft Apps Updater

Version 0.3.2 renames the app to Craft Apps Manager. Download this release manually: older versions may reject the renamed repository or package during their built-in update check. Extract the new package into your existing library folder to retain apps, sources, builds and settings. Open `CraftApps-Manager.exe`; the old startup preferences are migrated automatically. Close the old Updater first. If you used automatic app or source updates, disable those tasks in the old app before switching and enable them again in Manager.

## Supported apps

Every Craft app published under [github.com/storytold](https://github.com/storytold) is supported, including apps released after this version:

- **Release and source updates:** DesignCraft, EffectCraft, FilmCraft, LightCraft, PhotoCraft, PDFCraft, VectorCraft, WordCraft, GridCraft, DeckCraft, CADCraft and SoundCraft.
- **Releases only:** ArtCraft, whose Windows installers and macOS disk images use their own naming (`ArtCraft_<version>_…`). On Windows it is installed with its installer. Third-party apps are not included by default; add compatible repositories through Custom apps in Settings.
- **Source only:** ArtCraft X.

Apps hosted outside Storytold are listed by their `owner/name` repository, so their releases are checked, downloaded and updated like the rest. Their file names are matched case-insensitively, and the `x86_64`/`aarch64` architecture spellings are accepted alongside `x64`/`arm64`.

You can track your own apps too: paste a GitHub repository link in Settings → Apps → Custom apps. The manager verifies that the repository's latest stable release carries an installable file for your system, then checks, downloads and updates it like a built-in app.

Release formats and architectures depend on what each upstream app publishes.

The manager can discover new Craft apps from Storytold without a manager update. Use **Settings → Updates → App list → Refresh** to refresh the list manually. Startup discovery and discovery during hourly app checks are separate opt-in settings with a shared six-hour cache. New apps appear in the list but are not selected for bulk app/source updates unless you enable **Include newly published Craft apps automatically**. Discovery never installs apps. A built-in list and saved catalog keep the manager usable offline. An optional `GITHUB_TOKEN` (or `CRAFT_GITHUB_TOKEN`) raises the GitHub API limit; it is sent only to `api.github.com`.

PDFCraft's repository was renamed from PrintCraft. The manager accepts both `pdfcraft` and older `printcraft` release files and executable names, while retaining the existing library identity. More generally, when an app's release uses a single new file name, the manager installs it as the same app and remembers the new name. Installer mode detects apps installed before Manager, including on the first launch of a fresh library.

## Updating apps

The list on the left groups apps into **Installed** and **Not installed**; type in the search field to filter it. **Get** next to an app, or **Update** when a newer release is known, installs it after a confirmation. Click an app to open its page:

- **Install** downloads and installs that app using the format selected in Settings.
- **Open** starts the installed app. **Check for updates** checks one installed app; if a newer release exists, the main button becomes **Update to** that version.
- **Uninstall…** removes a managed portable copy or opens its MSI uninstaller. Other installer types use Windows Installed apps.
- **Launch options** in the tools column lets you choose the executable and add arguments, one per line.

The uninstall confirmation has an optional **Delete app profile data** checkbox, off by default. It lists the app-specific profile folders that can be removed after uninstall succeeds, including settings, caches, plug-ins and recovery/autosave copies. Some portable and installer copies share the same AppData profile. Custom profile locations outside the listed folders are kept. Portable PhotoCraft profiles are retained in `runtime/app-profiles` when the checkbox is off and restored when that portable app is installed again.

Installer is the default release format. Windows installer wizards may ask for administrator permission. Portable ZIPs are extracted into the app library. Portable and installer copies are tracked separately. Switching the release format selects the matching copy for Launch, update checks, and Uninstall; the other copy stays in place.

Home has **Update All**, with its app selection under **Settings → Updates**. It updates only installed apps and lists every step for confirmation; it never installs missing apps. Newly installed apps are included automatically. Manual source downloads and builds remain in each app’s tools.

Use the arrow beside Install or Check for updates to select another published version. Choosing an older version shows a **Downgrade** confirmation. Available versions depend on the app's published packages for your platform and architecture.

Right-click an app in the sidebar or Home and choose **Hide**. Restore hidden apps through **Settings → Apps → App visibility → Choose…**. Hiding an app does not uninstall it or change update preferences. A short explanation appears on the first Hide action in each session.

Enable **Keep running in system tray when closed** in General settings to keep checks running after closing the window. The tray icon appears only while the window is hidden and provides Open, Check for app updates and Exit. If no usable tray is available, closing exits normally.

**Move…** relocates managed portable apps and supported Windows MSI installations. The destination is checked before moving, and an open app must be closed. The manager updates its location and owned shortcuts, while project files outside the app folder stay untouched. Linux native packages and macOS installer copies keep their system-managed locations.

Under **Settings → Updates → Automatic update checks**, choose whether to check when the manager opens, periodically while it is open (10–60 minutes, default 20), or every hour even when closed using the system scheduler. The interval field is inactive when periodic checks are off. Save applies scheduler changes; Cancel leaves them unchanged. In-app periodic checks reuse recent scheduled results for the same package format and architecture. Checks never download or install updates automatically. Notifications are controlled separately.

Settings also has a separate check for this manager itself. Startup checks are off by default. Enable them to check for a newer stable release matching the manager's architecture when it opens; an **Update available** button then appears next to the version in the top bar. Nothing downloads until you confirm **Download and restart**. The package is verified against GitHub's published SHA-256 digest before a native helper replaces the EXE. Close other manager and builder windows first. Your library and settings stay in place, and the previous EXE is retained under `runtime/self-update` for recovery.

## Building from source

Open an app's page and use **Set up build tools** in its tools column before building, then **Build**. **Download the latest source first** fetches the newest source ZIP; turn it off to build from the local ZIP. The setup installs only the extra tools needed for that app. To build ArtCraft X or work in a separate window, use **Open builder window** under **Settings → Builds**. Source builds currently require 64-bit Windows, even with the x86 manager. App and source updates work with either manager architecture. Rust builds require Microsoft's C++ build tools; ArtCraft X also needs its frontend tools.

The builder uses the upstream source and lockfiles without dependency patches. Build output appears in `builds`; **Open folder** and **View log** in the Build row open the latest build and its log. Failed or canceled builds keep their cache so you can try again. Successful-build cleanup is configurable.

Upstream build failures can still happen. Warnings from an upstream project are shown in the log rather than hidden or patched away.

## Files and backups

On Windows, the app keeps its library beside the executable by default. On Linux, it uses `$XDG_DATA_HOME/craft-apps-manager`, or `~/.local/share/craft-apps-manager` when that variable is unset. On macOS, it uses `~/Library/Application Support/Craft Apps Manager`:

```text
releases/           Portable apps and downloaded installers
sources/            Source ZIPs and their index
builds/             Finished builds, grouped by app
logs/               Update and build logs
backups/releases/   Portable app backups
backups/sources/    Source backups
workspace/          Build tools, extracted sources, and build cache
runtime/            Download staging and internal state
```

You can change the library and build-tool locations in Settings. The change applies when you reopen the window.

Portable and source backups are optional. Compression uses 7-Zip LZMA2; archives are verified before the original backup is removed. Windows installer installations are not backed up by this app. Logs rotate at the configured size instead of creating a new file for every build.

Each app page has **Backups → Manage…** in its tools column, with Restore and Delete modes. Restore lets you choose one release or source backup by version or commit, date, and format. Delete lets you select individual backups or all of that app's backups. Both actions require confirmation. Restore replaces the current managed copy and keeps the selected backup. To restore a portable release, select Portable ZIP in Settings first. Source backups can be restored in either mode. These controls do not restore or remove Windows installer installations.

## Building this manager

Windows, Linux and macOS share one source checkout. Platform-specific code lives in `src/platform`, `src/installers`, `src/scheduler`, and `src/tools`; generated executables and packages are not part of the source repository.

For Windows, install stable Rust and Microsoft's **Desktop development with C++** workload, including its x86 and x64 tools. Build each target separately:

```text
rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc
cargo build --release --locked --target x86_64-pc-windows-msvc
cargo build --release --locked --target i686-pc-windows-msvc
```

Executables are under `target/<target>/release`. Distribution folders are `dist/windows-x64` and `dist/windows-x86`; each contains its matching executable and portable 7-Zip. The builder uses the same executable with `--builder`. An x86 manager defaults to x86 Craft app releases and only accepts x86 self-update packages. A saved release architecture preference still takes precedence.

```text
cargo test --locked
cargo clippy --all-targets -- -D warnings
```

Some integration tests require local build tools and 7-Zip. Set `CRAFT_TEST_TOOLS` to their parent folder to run those checks. The tests don't install or uninstall your real apps.

For Linux build commands, native prerequisites and RPM packaging, see [docs/linux.md](docs/linux.md). For the experimental macOS version, see [docs/macos.md](docs/macos.md). A system-installed Linux manager is updated through a newer DEB or RPM package rather than the portable ZIP replacement helper.

## Reporting problems

Open an issue with the app version, the Craft app involved, and the relevant part of `logs/updates.log` or that app's build log. Remove personal paths or anything private before posting it.

## License

MIT. The bundled 7-Zip files have their own license notices. Craft apps and their branding belong to their respective authors.

## Preparing a release

See [Release preparation](docs/releases.md) for the version-bump command and review process.
