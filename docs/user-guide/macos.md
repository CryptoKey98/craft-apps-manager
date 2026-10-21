# macOS

The manager supports macOS 11 or newer on Intel and Apple silicon. Universal downloads contain both architectures. macOS support remains experimental.

## Install the manager

Choose a download from [Releases](https://github.com/CryptoKey98/craft-apps-manager/releases/latest):

- **DMG:** open it, drag **Craft Apps Manager** into **Applications**, then launch the copied app from Applications. Eject the mounted disk image afterward. Finder handles the copy and progress display.
- **Portable ZIP:** extract the whole archive into a writable folder. Keep `portable.txt` beside `Craft Apps Manager.app`; launch the app from that folder.

A DMG is a disk image, not an installer wizard. Do not run the manager from the mounted DMG when testing updates.

## First launch

The manager has an ad-hoc signature and is not Apple-notarized. If macOS blocks the downloaded app, dismiss the warning, open **System Settings → Privacy & Security**, and use **Open Anyway** when available for that app. Follow macOS's confirmation prompts. Do not disable system-wide security checks.

Rust and 7-Zip are not required to run the manager. Compilation prerequisites are in [Building and testing](../development/building.md#macos).

## Apps and saved data

For supported Craft app DMGs, the installation dialog offers:

- **Installer:** copies the app into `/Applications`, or `~/Applications` when `/Applications` is not writable.
- **Portable:** keeps that app in the manager's library.

Download integrity and bundle verification still apply. Custom app support depends on the upstream download and bundle identity. Uninstall removes the selected app bundle; app profile deletion is not supported on macOS.

Installed managers normally use `~/Library/Application Support/Craft Apps Manager`. Portable managers use portable storage, with new libraries in `data` beside the app. The Data path identifies your actual library. See [Storage and portability](using-the-manager.md#storage-and-portability).

Hourly scheduled checks use launch agents in `~/Library/LaunchAgents/io.github.craft-apps-manager.*.plist`. Checks notify through Notification Center and do not automatically install updates.

## Manager updates

The updater checks the offered package's identity and integrity before replacement. Service-enabled builds can use supported ZIP or DMG payloads; older GitHub-only builds use their compatible update path. A supported archive-format change does not itself require manually changing your installation.

Close active work before updating. Keep the app in a normal installed or portable location, not on a mounted DMG. A development executable outside an app bundle cannot use the bundle self-update flow. Protected locations may require administrator approval.

Staging and rollback stay beside the installed bundle even if the library is on another volume. Failed replacement attempts recovery and records the result. macOS accepting a restart request does not prove the app finished starting.

### Older updater failures

Managers 0.8.1–0.8.3 could fail when another app in the same directory contained symbolic links, with `Archive contains a linked path`. The saved failure could also reappear on launch. These issues were fixed in 0.9.0.

If an affected updater still fails, manually download a current macOS package and replace the manager app, preserving your library and portable files. The saved error may appear once on the first fixed launch; dismiss it. Users already on 0.9.0 have the corrected updater.

Failures can leave a `.craft-manager-update-*` recovery folder beside the app. Check the error and retained `previous.app` before removing anything. Results may also be recorded in `~/Library/Application Support/Craft Apps Manager/self-update-result.json` and the staging folder's `result.json`. See [Updater recovery](../development/updater-operations.md#failure-recovery-and-retention) before deleting recovery data.

## Builds and backups

Manager and Builder are available in both ZIP and DMG packages. Building Craft apps requires Rust, Xcode Command Line Tools, and app-specific tools. Native source ZIP extraction is internal and keeps archive safety checks; 7-Zip is optional.

7-Zip is needed for compressed `.7z` backups and restoring existing `.7z` backups. Without it, the original backup is retained. Install `sevenzip` through Homebrew if needed. ArtCraft X needs additional tools such as Node, CMake, and NASM; supported setup can use Homebrew for those tools. Local app builds produce executables rather than complete app bundles.

Shortcut files and app-profile deletion are not provided on macOS. Open app bundles directly in Finder.

[Using the manager](using-the-manager.md) · [Documentation index](../README.md)
