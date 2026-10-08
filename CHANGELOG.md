# Changelog

## 0.3.0

### Added

- An app-specific Backups window with restore and delete modes, backup details, and confirmation prompts.
- Updater update checks in Settings, an optional startup check, and verified download-and-restart updates.
- Current updater version and build information in Settings.
- Optional app profile cleanup after uninstall, with the exact folders shown before confirmation.
- Portable PhotoCraft profile preservation and restoration when reinstalling.
- A clear OK popup when no release or source apps are selected.

### Fixed

- Stale installer and portable records no longer mark missing apps as installed or enable Uninstall.
- MSI uninstall uses the current registered product code.
- Working text is centered; unknown progress bounces smoothly while known percentages fill normally.
- Backups is disabled when the selected app has no backups.
- Automatic app updates are disabled for installer mode, with an explanatory tooltip. Source updates remain available.
- Task Scheduler integration uses the Windows API. Opening the updater only reads task status and does not change scheduled tasks.

## 0.2.0

- Keep separate records for portable and Windows installer copies of each app.
- Restore the matching installation when switching release formats in Settings.
- Use the selected copy for launching, update checks, and uninstalling.
- Recover existing portable copies from their app folders and executable version information where available.

## 0.1.0

First release: app and source updates, Windows installers and portable ZIPs, source builds, automatic updates, and optional compressed backups.
