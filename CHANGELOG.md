# Changelog

## 0.6.0

Fork release from [carpaaaaa/craft-apps-manager](https://github.com/carpaaaaa/craft-apps-manager), based on 0.5.1.

- Read the app list from the Storytold repositories on GitHub, so Craft apps published after this version appear without a manager update. A built-in list is used offline.
- Add newly found apps to Update all and Update all sources automatically (can be turned off in Settings) and announce them in hourly checks.
- Add Settings → App list → Refresh to look for new apps at any time.
- Support ArtCraft: Windows installers and macOS disk images named `ArtCraft_<version>_…`, release tags such as `artcraft-v0.41.0` and its `ai.artcraft.app` bundle identifier.
- Accept a release whose files switched to a single new name and remember that name, so renamed apps keep installing and updating.
- Download icons for apps without a bundled one, with a neutral tile when a repository has none.
- Accept an optional `GITHUB_TOKEN` (or `CRAFT_GITHUB_TOKEN`) to raise GitHub's API limit.
- Add `--list-apps`, `--install-app`, `--uninstall-app` and `--release-format` for scripts and end-to-end checks.
- Self-update from carpaaaaa/craft-apps-manager releases.

## 0.5.1

- Add a saved light and dark theme with a slim toggle beside Settings.
- Blend interface colors smoothly when switching themes.
- Use Adobe Spectrum neutral grays in light and dark themes.
- Add an About section with project information and contributor credits.
- Add experimental Arch Linux package support and packaging tools.
- Add local build launch controls, saved launch arguments and a build options window.
- Allow local builds to be deleted with confirmation.

## 0.5.0

- Redesign the manager window: an app list with search and Installed / Not installed groups, an Overview page for Update all and Update all sources, and a page for each app with its description, actions, details and tools.
- Build from source, download an app's source, edit launch options and manage backups from the app page; the separate builder window remains for any source, including ArtCraft X.
- Show available manager updates as a button in the top bar.
- Combine manager and builder settings in one scrolling Settings window with a section list.
- Restyle every dialog and use IBM Plex Sans and Plex Mono on all platforms.
- Keep the app list current while a build runs, label backup restores and deletions correctly, and explain why disabled buttons are unavailable.

## 0.4.2

- Prefer the newest installed PDFCraft/PrintCraft version on Windows when both names are registered.
- Resolve renamed app executables correctly on Linux and macOS, including saved launch settings.
- Improve macOS app replacement and manager update recovery across different volumes.
- Review selected release updates before downloading and improve settings recovery, cancellation, and keyboard accessibility.
- Add a local version preparation tool and CI validation for release metadata.
- Avoid duplicate workflow runs for pull request branches.

## 0.4.1

- Add WordCraft, GridCraft, DeckCraft, CADCraft, and SoundCraft to release updates, source updates, and the source builder on supported platforms.
- Include the upstream app icons and preserve existing app selections and ordering when expanding the catalog.
- Show app and source selection totals from the current catalog.
- Add experimental macOS support contributed in pull request #2.

## 0.4.0

- Detect previously installed apps on the first launch of a fresh library.
- Accept installer names with version or architecture suffixes.
- Support renamed PDFCraft release assets and executables alongside older PrintCraft names.
- Disable Open Log until a log exists and show errors when opening it fails.

- Add Windows x64 and x86 MSI installers alongside portable ZIPs.
- Keep MSI user data outside the installation folder and preserve it during upgrades and uninstall.
- Select MSI or ZIP manager updates according to installation type.
- Add Linux x86 builds, architecture-aware RPM dependencies, and portable ZIP packaging.
- Add a Linux platform layer shared with the Windows application.
- Support AppImage releases and native DEB/RPM installation and removal.
- Add systemd user timers for hourly availability checks and desktop notifications.
- Support Linux source building, including prerequisite setup through apt or DNF.
- Add Ubuntu DEB and Fedora RPM packaging with desktop launchers and license notices.
- Fix Linux shortcut names, portable uninstallation and builder launch after executable replacement.
- Explain how to update a system-installed Linux manager before attempting a portable self-update.

Linux testing covers Ubuntu 26.04 and Fedora 44 on x86_64, with x86 manager and builder launch checks on both multilib hosts. Native 32-bit operating systems have not been tested. Linux packages are experimental and require glibc 2.43 or newer; older distributions and ARM64 are not verified.

## 0.3.3

- Replace automatic app and source downloads with hourly availability checks.
- Support hourly checks for both installer and portable apps.
- Notify once per new app version or downloaded source commit; downloads require confirmation.
- Show saved app update availability when opening Manager.
- Keep existing scheduled background commands check-only after upgrading.
- Forward Cancel update to the installer wizard and wait for its result; show guidance when Windows blocks the request.


## 0.3.2

- Rename the project to Craft Apps Manager and add an engraved up-arrow application icon.
- Allow dragging sidebar apps into a saved custom order.
- Keep app rows stable during update checks and allow scrolling in smaller windows.
- Make sidebar install and update buttons open confirmation directly.
- Improve update-arrow contrast, hover glow, and sidebar footer spacing.
- Migrate existing startup preferences to the renamed settings file.
- Add optional startup availability checks for installed apps, separate from this program's own version check.
- Show a pulsing circular update icon in the sidebar when an installed app has a newer release.
- Keep startup app checks read-only; downloads and installation still require confirmation.

- Move app controls beside the Creative Apps list, expanding toward the update section.
- Use official upstream app icons, clearer status colors, and consistent button spacing.
- Add a gray circular install button with a recessed arrow beside Not installed.
- Clip fixed-size app controls during animation to avoid artifacts at the closing edge.
- Include upstream icon license notices in distributions.

## 0.3.1

### Added

- Windows x86 package with matching portable 7-Zip.
- Architecture-specific manager downloads and x86 release defaults.
- Automated checks for both Windows architectures.

### Fixed

- Installation status and backup discovery run in the background instead of blocking window repaints.
- App status refreshes after install/uninstall and release-format changes.
- Windows installer process-handle access works on x86.
- An embedded Windows manifest prevents unexpected installer-detection elevation prompts when opening the manager.

Source builds still require 64-bit Windows. The x86 app was tested on 64-bit Windows; native 32-bit Windows has not yet been tested.

## 0.3.0

### Added

- An app-specific Backups window with restore and delete modes, backup details, and confirmation prompts.
- Manager update checks in Settings, an optional startup check, and verified download-and-restart updates.
- Current manager version and build information in Settings.
- Optional app profile cleanup after uninstall, with the exact folders shown before confirmation.
- Portable PhotoCraft profile preservation and restoration when reinstalling.
- A clear OK popup when no release or source apps are selected.

### Fixed

- Stale installer and portable records no longer mark missing apps as installed or enable Uninstall.
- MSI uninstall uses the current registered product code.
- Working text is centered; unknown progress bounces smoothly while known percentages fill normally.
- Backups is disabled when the selected app has no backups.
- Automatic app updates are disabled for installer mode, with an explanatory tooltip. Source updates remain available.
- Task Scheduler integration uses the Windows API. Opening the manager only reads task status and does not change scheduled tasks.

## 0.2.0

- Keep separate records for portable and Windows installer copies of each app.
- Restore the matching installation when switching release formats in Settings.
- Use the selected copy for launching, update checks, and uninstalling.
- Recover existing portable copies from their app folders and executable version information where available.

## 0.1.0

First release: app and source updates, Windows installers and portable ZIPs, source builds, automatic updates, and optional compressed backups.
