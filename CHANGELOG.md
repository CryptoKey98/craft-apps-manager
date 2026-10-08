# Changelog

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
