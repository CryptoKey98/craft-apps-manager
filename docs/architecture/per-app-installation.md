# Per-application installation configuration

## Architecture and behavior

`src/installations/options.rs` owns installation intent, repository identities, host compatibility, installed-fact resolution, and release compatibility matrices. It reuses the existing Craft/Tauri asset matcher and generic GitHub download classifier rather than adding a second installation implementation.

Preferences persist in `manager-settings.json` under `appInstallOptions`, keyed by the lowercase GitHub owner/repository. Each entry contains `releaseFormat` and `architecture`. Installed records remain separate facts; selecting an option does not rewrite an existing installation's type or architecture. Choices persist immediately in the install dialog, including when the dialog is subsequently cancelled. Custom-app setup preferences are staged until Settings is saved.

The install dialog offers only confirmed host-compatible release combinations. Changing format recalculates architecture choices. Changing version reloads the selected release's combinations and preserves a valid choice when possible. Single available combinations select automatically. Windows x64 supports x64 and x86; other hosts conservatively offer native architecture. Universal macOS assets can satisfy the native architecture. Existing portable/AppImage/native installer execution, checksums, destinations, shortcuts, cancellation, and progress paths are reused.

Built-in and custom applications use the same options resolver and release planner. Custom apps no longer disappear because a library-wide format changed. Existing custom download confirmation remains available for ambiguous packages and incomplete naming; unlabeled architectures are not silently guessed.

Global format/architecture controls were removed from Settings. Old JSON fields remain readable as legacy/context fields but no longer determine library installations. Other settings remain intact. CLI format/architecture arguments now require a specific install target and save only that application's preference.

Bulk updates and background checks derive each app's configuration from its verified installed record. Each planned entry carries its own immutable execution preferences. Missing compatible downloads become per-app errors, without switching packages or architecture, downloading replacements, or blocking unattended operation for a dialog. Explicit version selection retains downgrade behavior. Changing the type or architecture of an existing installation requires uninstalling that installation first, preventing silent overwrite and native-installer conflicts.

Existing installations remain visible. Existing reliable type/architecture metadata is retained, and missing binary architecture can be recovered from PE or ELF headers. Truly unknown configuration fails safely for updates rather than borrowing obsolete global values. Unknown legacy macOS architecture cannot currently be recovered from Mach-O bundles. Existing relocation, removal, backups, source builder, installer cancellation, and portable extraction logic is preserved.

## Multiple installations

`Config.apps` stores the selected copy for each app; `Config.installations` retains copies by app, format, and location rather than collapsing copies with the same architecture. Migration inventories native installs and the default portable library independently. A verified saved selection survives changes to pending package preferences and restarts.

The app-page Installation dropdown switches the selected copy and its action context. Add another installation opens the existing install dialog using a separate, validated plan; portable copies require unused, non-overlapping folders, and a second native installer copy is rejected. Previewing or cancelling does not remove or replace the selected installation. Update All and background updates target the selected copy.

Home's compact installation switcher uses the same selected copy. Portable copies receive matching letters in both selectors and desktop shortcuts. Existing letters stay attached to their copies; a new copy reuses the first free letter without renaming surviving copies. Legacy records receive labels when loaded. Versions in selectors use a `v` prefix, with the letter at the end in parentheses.

Additional portable copies in the default library use flat numbered folders such as `releases/filmcraft-2`. Existing folders retain their paths. Uninstall removes empty legacy manager-created wrapper folders, preserving user-selected parent folders and folders containing other files. New installs and moves create requested desktop shortcuts without extra library shortcuts.

The install dialog includes a Version dropdown for first installs and additional copies. Selecting a version from the app page preselects its release tag. Explicit versions remain pinned while changing compatible installation options; installing an older additional copy does not downgrade the existing copy.

Uninstall removes only the selected copy's record and selects a remaining verified copy. Lost legacy records can be recovered from the default portable library; unknown external folders cannot be rediscovered without their saved location. Launch options are scoped by app and installation path, with migration when a copy moves. Different copies may still share application-managed user settings.

## Source files

Added `src/installations/options.rs`.

Modified `src/model.rs`, `src/catalog.rs`, `src/updates.rs`, `src/hourly.rs`, `src/settings.rs`, `src/main.rs`, `src/lib.rs`, `src/ui/mod.rs`, `src/ui/dialogs.rs`, `src/ui/settings.rs`, `src/ui/app_page.rs`, `src/ui/sidebar.rs`, `src/ui/overview.rs`, and `tests/workflows.rs`.

## Validation and compatibility

Tests cover independent preference persistence, actual release combinations, legacy settings, installed facts overriding preference, mixed bulk updates, unavailable updates, version changes, cancellation, destination safety, removal, backups and existing builder workflows. See [Operations](operations.md) for current test counts and manual validation requirements.

Changing package options uses cached metadata and preserves preview details while recalculating. Approval of stale plans is prevented. Installed package labels come from verified records rather than a future preference. Release versions support two-, three- and four-part numeric versions and supported timestamp tags; missing numeric components compare as zero.

Older settings remain readable. Unknown installed architecture is not silently replaced with a global preference. Legacy macOS Mach-O architecture recovery remains a limitation. Custom installer behavior still depends on what publishers register with the operating system.
