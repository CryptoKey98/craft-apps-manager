# Using the manager

Start with the [quick start](../../README.md#get-started). This guide covers everyday use; [building the manager](../development/building.md) is separate.

## Installations

- **Install options belong to each app.** Choose a version, release format, and architecture in its installation window. Only compatible combinations are offered; there are no global format or architecture controls in Settings.
- **Open, update, or uninstall** from the app page. Launch options let you select an executable and add arguments.
- **Keep multiple installations.** After the first install, use the **Installation** dropdown to select the copy you want to manage. **Add another installation…** opens the usual install window. Portable copies need separate folders; one native installer copy is supported. Apps may share their own user settings between copies.
- **Identify portable copies by number.** The first copy is unnumbered; extra copies use matching numbers in installation selectors and desktop shortcuts. Removing copy 2 leaves the first copy and copy 3 unchanged; the next new copy uses 2. Moving a copy preserves its existing folder name. Home's compact installation switcher shares the app page's selection.
- **Move a copy** with **Move…** where supported: managed portable apps and supported Windows MSI installations. Linux native packages and macOS installer copies keep their system-managed locations.
- **Hide apps** from the sidebar or Home context menu. Restore them in **Settings → Apps → App visibility**. Hiding does not uninstall an app or change its update selection.

Updates and version changes use the selected installation's verified format and architecture. Unknown installation details are reported rather than guessed. Changing an existing copy's format or architecture requires removing that copy first, or adding a separate supported installation.

New app installations start with Portable when using a portable manager, or Installer when using an installed manager. Your saved choices for each app take priority, and only available package types are offered. Native app locations are chosen by the installer or system package; the install dialog shows a read-only location. Updates, downgrades, and rollback preserve the verified installation scope and folder. Versions verified as incompatible are disabled, and their information button explains why. Existing equivalent Windows desktop shortcuts are reused instead of creating duplicates.

## Updates and Activity

**Check for updates** checks one app. **Update All** on Home updates the selected installed apps after a review; it never installs missing apps. Newly installed apps are selected automatically. With multiple copies, updates target the selected copy of each app.

In **Settings → Updates**, configure startup checks, periodic checks while the manager is running, or hourly scheduled checks. Checks do not automatically download or install updates. The manager also checks for its own updates during periodic checks and displays **Update available** in the header when a matching release is available.

**Activity**, beside Settings, shows progress, cancellation, and results. Manager and Builder share operation records. Closing a dialog does not cancel its operation; use its Cancel button. Closing the manager during an active operation requests cancellation and waits for it to stop, unless close-to-tray hides the window instead. Native package transactions may need to finish before cancellation can be confirmed.

To hide the Activity button's notification badge, enable **Hide activity badge** in **Settings → General**. Unread activity highlights still appear inside the panel.

**Clear activity** permanently deletes finished activity records and their notification markers in Manager and Builder. Running tasks and batches that still have active work are kept, and new completions appear normally. Without clearing, older finished history is limited to 100 records; records still needed by active work are protected.

**Approve administrator access once per batch** is enabled by default for eligible Windows and Linux installer batches. Linux uses PolicyKit with apt, dnf, or pacman. Individual installers can still show their own prompts. Eligible manager bridge updates can also reuse one administrator approval on macOS; the initial installation is separate.

Platform-specific manager hotfixes use build revisions. Only affected platforms are offered an update; About shows `Rev. 2` or higher when installed.

**What's New** appears on first launch and after a version or revision change. Reopen it with the info icon beside the manager version.

## Installer cleanup

Downloaded setup files are automatically removed after a successful, verified installation or update. Cleanup appears in the operation's progress and Activity details.

Older setup files left by previous versions are not automatically swept up. Recovery packages or files required for installer maintenance may be retained. Failed or cancelled installations do not run successful-install cleanup; deletion problems are reported without marking a verified installation as failed.

The manager also removes an app's empty setup-download folder after cleanup. Additional portable library copies use flat folders such as `filmcraft-2`; uninstalling removes empty legacy wrapper folders while preserving user-selected parents and folders containing other files.

## Run in the background

Each data folder can have one Manager window and one Builder window open at a time. Opening either again brings its existing window back, including when the Manager is hidden in the tray. Separate portable libraries can run independently.

Enable **Keep running in system tray when closed** in General settings to hide the window while checks continue. The tray offers **Open**, **Check for app updates**, and **Exit**.

This needs a usable desktop tray. On Linux, some GNOME desktops require a compatible tray extension. If there is no tray host, closing quits normally so the manager cannot become inaccessible.

Linux close-to-tray also needs X11 or XWayland. The manager prefers a reachable XWayland display when available. On native Wayland, the setting is disabled and closing quits normally.

## GitHub activity and tokens

Open the GitHub control in the bottom-left corner to see the latest connection and token status. Cached actions do not need another API request. A rate-limit response may temporarily prevent fresh information from loading while already saved information remains usable.

**Manage token** opens the token section in Settings. The help link explains how to create a classic token. Add, replace, or remove a token, then use **Save** to apply the pending change; **Undo** cancels it. Tokens stay in the operating system's credential store.

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

New Storytold Craft apps can be discovered without updating the manager. Explicitly supported external sources include **tangency-labs/solidcraft**. Use **Settings → Updates → App list → Refresh**. Startup discovery is enabled by default. Discovery adds apps without installing them or selecting them for updates.

The **New** badge lasts up to 14 days from an app's official addition to discovery. Viewing the app clears it sooner. Repository age, app updates, cache clearing, and fresh manager installs do not restart that shared period. New apps initially appear at the top; drag them to change the sidebar or Home order independently.

When an app-release catalog is configured, the manager uses verified catalog information for supported Craft apps' version lists and package choices. Missing or unavailable catalog information falls back to GitHub automatically. Custom GitHub apps go directly to GitHub. Installation still verifies the selected download; browsing the version list does not download installers to inspect them.

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
rollbacks/          Verified previous versions, one per portable installation
backups/            Preserved legacy app and source backups
workspace/          Build tools, extracted sources, and build cache
runtime/            Staging, operation records, and migration recovery copies
```

`runtime/api-cache` stores reusable GitHub responses in compact, compressed files. The manager cleans up responses unused for 30 days and evicts older entries when this cache exceeds 20 MiB. Cleanup runs in the background; active entries can temporarily keep it above that limit. This does not remove installed apps, settings, builds, backups, or update recovery records.

Use **Settings → Updates → GitHub Cache → Clear cache**, below GitHub token, to remove saved GitHub information when troubleshooting. Progress and results appear inside the cache row. This is an immediate confirmed action; it does not require Settings Save. New requests refill the cache as needed. This also clears saved app-release catalog data when that feature is configured; it does not reset badge dates or dismissals.

Older API cache entries are converted only after the new copy has been checked. Their original timestamps are retained; corrupt or unsupported entries are ignored and fetched again when needed.

Recently used responses also stay in memory for quick access. Update checks reuse results for up to 60 seconds, version lists and package options for 15 minutes, and older-release details and repository descriptions for 24 hours. A changed release refreshes related version lists and previews. Before installing, the manager checks the reviewed download against release information no more than a minute old; package verification still applies.

Startup discovery and manual app-list refresh share a 60-second successful-check cooldown. When enabled, periodic discovery revalidates at the configured check interval. Browsing the saved app list does not trigger discovery, and a failed discovery does not reset its successful-check time. Cache freshness and the 30-day unused-file cleanup are separate.

`installations.json` records installed copies. `config.yaml` holds the library's manager and Builder preferences, custom apps, and launch choices. The small `data-root.json` file tells the manager where to find the library and tools. Folder changes apply after restarting.

Changes inside Settings remain pending until **Save**, including GitHub token additions and removals. **Cancel**, the close button, or Escape discard pending edits. Confirmed cleanup actions and update checks run separately.

Older settings are imported automatically. After verifying the saved configuration, the manager removes the unchanged old preference files. Locked, changed or unverified files are kept with a warning; cleanup can retry on the next launch. Do not remove installation records or migration files blindly. YAML preserves configuration values, but saving may normalize formatting and remove comments. Tokens remain in the system credential store, outside the configuration file.

</details>

## Build apps and roll back updates

Use an app's tools to **Set up build tools**, download its source, and **Build** or **Rebuild**. Open the separate Builder through **Settings → Builds → Open builder window**. Build Options lets you choose which completed build the Launch button uses; that choice is shared between Manager and Builder.

Builds use upstream source and lockfiles. Upstream compilation failures are shown in the log. Required tools depend on the app and platform; Windows Rust builds need Microsoft's C++ build tools, and ArtCraft X also needs frontend tools. Failed or cancelled builds keep their cache for a retry.

**Build Options** can delete temporary cache from interrupted builds, even before a successful build exists. Cleanup is unavailable during a build and preserves completed builds and source ZIPs. The next build recreates its cache.

Use **Settings → Updates → Rollback → Keep the previous version for rollback** to retain one uncompressed previous version for supported portable installations. New libraries enable this by default; existing libraries preserve their previous backup-retention preference. The old compression and source-backup controls are retired.

Open **Rollback → Review…** in an app’s Tools section. Click the previous-version card, then **Confirm**, to return that selected installation to the retained version. Each portable installation has its own previous version, even when several copies of the app are installed. **Remove previous version…** asks before deleting that retained copy.

The Rollback dialog can inherit the global setting or use **Enabled** / **Disabled** for that installation. Changes apply only with **Save**; **Cancel** discards them. Disabling retention affects future updates and does not delete an existing previous version. Uninstalling a portable installation also removes its verified retained version; other installations and legacy archives are preserved.

Rollback changes program files, not the format of your documents. Older versions may not understand newer settings or app data. Keep separate backups of important work. Native installations use the existing verified installer version selector rather than folder snapshots; there is no universal installer rollback. Temporary recovery protection during replacement remains active even when retaining previous versions is disabled.

New updates no longer create compressed app or source backups. Existing legacy backup files remain on disk; the manager does not guess which installation owns an old archive or automatically delete it during migration. Open those archives separately with a compatible archive utility if you need their contents.

Uninstalling can optionally remove listed app profile folders. This is off by default, and some installations share profiles. Review the confirmation before deleting profile data.


## Troubleshooting

Activity shows operation results and access to logs. Include the manager version/revision, operating system, app, installation type, steps, and exact error when reporting a problem. For detection problems, maintainers may ask for `installations.json` from the active Data folder. Redact personal information and never share tokens or passwords.

A custom repository must be active, not a fork, and have a stable compatible desktop release. CLI-only repositories are rejected. Some downloads need an explicit architecture or application choice. An API token can help with rate limits; it cannot add an upstream package that does not exist.

### Upgrading from Craft Apps Updater

The app was renamed in 0.3.2. Very old Updater versions may reject the renamed repository or packages and need a manual manager download. Close the old Updater and preserve its library when switching. Disable its old scheduled tasks before enabling scheduled checks in Manager.

[Documentation index](../README.md)

Libraries from older managers import `settings.json` into `installations.json` when the new inventory is absent. The library location and inventory must validate before the old file is removed. If both exist, `installations.json` is authoritative; a damaged new inventory is reported rather than replaced with old data. Operational installation records remain JSON. Older manager versions do not understand the new inventory filename.
