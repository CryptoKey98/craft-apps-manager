# Using the manager

Start with the [quick start](../../README.md#get-started). This guide covers everyday use; [building the manager](../development/building.md) is separate.

## Installations

- **Install options belong to each app.** Choose a version, release format, and architecture in its installation window. Only compatible combinations are offered; there are no global format or architecture controls in Settings.
- **Open, update, or uninstall** from the app page. Launch options let you select an executable and add arguments.
- **Keep multiple installations.** After the first install, use the **Installation** dropdown to select the copy you want to manage. **Add another installation…** opens the usual install window. Portable copies need separate folders; one native installer copy is supported. Apps may share their own user settings between copies.
- **Identify portable copies by letter.** The installation selectors and desktop shortcuts use matching letters. Removing B leaves A and C unchanged; the next new copy uses B. Home's compact installation switcher shares the app page's selection.
- **Move a copy** with **Move…** where supported: managed portable apps and supported Windows MSI installations. Linux native packages and macOS installer copies keep their system-managed locations.
- **Hide apps** from the sidebar or Home context menu. Restore them in **Settings → Apps → App visibility**. Hiding does not uninstall an app or change its update selection.

Updates and version changes use the selected installation's verified format and architecture. Unknown installation details are reported rather than guessed. Changing an existing copy's format or architecture requires removing that copy first, or adding a separate supported installation.

## Updates and Activity

**Check for updates** checks one app. **Update All** on Home updates the selected installed apps after a review; it never installs missing apps. Newly installed apps are selected automatically. With multiple copies, updates target the selected copy of each app.

In **Settings → Updates**, configure startup checks, periodic checks while the manager is running, or hourly scheduled checks. Checks do not automatically download or install updates. The manager also checks for its own updates during periodic checks and displays **Update available** in the header when a matching release is available.

**Activity**, beside Settings, shows progress, cancellation, and results. Manager and Builder share operation records. Closing a dialog does not cancel its operation; use its Cancel button. Closing the manager during an active operation requests cancellation and waits for it to stop, unless close-to-tray hides the window instead. Native package transactions may need to finish before cancellation can be confirmed.

To hide the Activity button's notification badge, enable **Hide activity badge** in **Settings → General**. Unread activity highlights still appear inside the panel.

**Approve administrator access once per batch** is enabled by default for eligible Windows and Linux installer batches. Linux uses PolicyKit with apt, dnf, or pacman. Individual installers can still show their own prompts. Eligible manager bridge updates can also reuse one administrator approval on macOS; the initial installation is separate.

Platform-specific manager hotfixes use build revisions. Only affected platforms are offered an update; About shows `Rev. 2` or higher when installed.

**What's New** appears on first launch and after a version or revision change. Reopen it with the info icon beside the manager version.

## Installer cleanup

Downloaded setup files are automatically removed after a successful, verified installation or update. Cleanup appears in the operation's progress and Activity details.

Older setup files left by previous versions are not automatically swept up. Recovery packages or files required for installer maintenance may be retained. Failed or cancelled installations do not run successful-install cleanup; deletion problems are reported without marking a verified installation as failed.

The manager also removes an app's empty setup-download folder after cleanup. Additional portable library copies use flat folders such as `filmcraft-2`; uninstalling removes empty legacy wrapper folders while preserving user-selected parents and folders containing other files.

## Run in the background

Enable **Keep running in system tray when closed** in General settings to hide the window while checks continue. The tray offers **Open**, **Check for app updates**, and **Exit**.

This needs a usable desktop tray. On Linux, some GNOME desktops require a compatible tray extension. If there is no tray host, closing quits normally so the manager cannot become inaccessible.

Linux close-to-tray also needs X11 or XWayland. The manager prefers a reachable XWayland display when available. On native Wayland, the setting is disabled and closing quits normally.

## Custom GitHub apps

Paste a repository link into **Settings → Apps → Custom apps**. The setup dialog lets you review the detected download, package type, and architecture, and choose a download when several are possible.

Adding a repository adds it to the list; it does not install the app. Removing it from the list does not uninstall it. A repository needs a compatible downloadable release for your system.

The manager remembers verified package identities and your selected executable, even when their names differ from the GitHub repository. If a package contains several possible apps, choose the one you want to use. Background updates stop with an explanation when a new choice is required.

If a Windows installer leaves an existing program unchanged, you can explicitly select its verified registration. Supported Linux desktop launchers retain their arguments and per-app environment; arbitrary shell commands are not evaluated.

Linux native packages need a package-owned desktop launcher. Unsupported installer types or unusual launch wrappers may still need manual installation.

**Custom-app support is experimental.** Asset naming and installer registration vary, so some apps may not install, be detected, or uninstall reliably. Third-party apps are not included by default.

An optional GitHub token can be saved under **Settings → Updates** to raise the API limit. It is stored in the operating system's credential store. Environment variables take priority over the saved token: `CRAFT_GITHUB_TOKEN`, then `GITHUB_TOKEN`. Authentication is sent only to `api.github.com`; don't post tokens in issues or logs.

## Craft app discovery

The manager includes the main Craft apps, including DesignCraft, EffectCraft, FilmCraft, LightCraft, PhotoCraft, PDFCraft, VectorCraft, WordCraft, GridCraft, DeckCraft, CADCraft, and SoundCraft. ArtCraft has release support; ArtCraft X has source-build support. Available downloads depend on what each app publishes for your platform.

New Storytold Craft apps can be discovered without updating the manager. Use **Settings → Updates → App list → Refresh**. Startup discovery is enabled by default and uses a shared six-hour cache. Discovery adds apps to the list without installing them or selecting them for updates. The New badge is limited to recently created apps; a fresh manager installation does not make an old app new.

Older PrintCraft files remain compatible with PDFCraft's library identity. App discovery and ArtCraft support were contributed by [carpaaaaa](https://github.com/carpaaaaa).

## Storage and portability

Portable archives include `portable.txt` and use portable storage automatically. A new portable library lives in a `data` folder beside the manager. Existing external libraries are preserved during migration.

To move a portable manager, finish active operations, close **both Manager and Builder**, and move the **whole extracted folder**. Keep `portable.txt`, `portable-state.json`, and bundled resources with it. Managed paths inside that folder follow the move; external locations remain unchanged.

Native installations, system-stored tokens, and desktop shortcuts do not move with the folder. Shortcuts may need recreation, and individual apps may store preferences elsewhere. Moving between operating systems does not convert apps or builds. Don't run an older manager against migrated portable metadata.

| Manager installation | Default library location |
| --- | --- |
| Portable archive | `data` inside the portable folder; existing external libraries are retained |
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


## Troubleshooting

Activity shows operation results and access to logs. Include the manager version/revision, operating system, app, installation type, steps, and exact error when reporting a problem. For detection problems, maintainers may ask for `settings.json` from the active Data folder. Redact personal information and never share tokens or passwords.

A custom repository must be active, not a fork, and have a stable compatible desktop release. CLI-only repositories are rejected. Some downloads need an explicit architecture or application choice. An API token can help with rate limits; it cannot add an upstream package that does not exist.

### Upgrading from Craft Apps Updater

The app was renamed in 0.3.2. Very old Updater versions may reject the renamed repository or packages and need a manual manager download. Close the old Updater and preserve its library when switching. Disable its old scheduled tasks before enabling scheduled checks in Manager.

[Documentation index](../README.md)
