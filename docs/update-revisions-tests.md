# Local revision validation — 0.8.3

Validated on October 14, 2026. No native packages were installed and no production settings or applications were changed.

| System | Successful checks |
| --- | --- |
| Windows x64 | Release build; formatting; all-target/all-feature Clippy with warnings denied; 264 Rust tests passed and one ignored; revision-1 binary identity; publishing and packaging PowerShell syntax; workflow actionlint |
| Ubuntu | Revision-1 and revision-2 Linux binaries built; four Rust revision tests; five publishing tests; real DEB built with version `0.8.3-2`; compatibility DEB with version `0.8.3`; both extracted executables reported revision 2 and matched the original binary; native version ordering; stale-binary packaging rejection |
| Fedora | Linux revision-1 and revision-2 binaries executed; real revision-2 RPM built; four publishing tests passed (Debian-only test skipped); signed RPM manifest and checksum; compatibility alias matched canonical package; extracted executable reported revision 2 |
| Arch | Linux revision-1 and revision-2 binaries executed; real revision-2 Arch package built; four publishing tests passed (Debian-only test skipped); signed Arch manifest and checksum; compatibility alias matched canonical package; extracted executable reported revision 2 |

Fedora and Arch used the Ubuntu-built Linux test executables to validate runtime identity and their native packaging flows. Their Rust codebases were not rebuilt independently during these checks. Package extraction and identity queries do not validate native installation transactions.

Automated cases cover signature tampering, expiration, repository mismatch, missing or modified assets, skipping intermediate revisions, unchanged platforms, preventing downgrade offers, retaining existing platform entries, and rejecting non-advancing revisions or unrelated platform uploads.

Local evidence is retained under `target/investigation/`. The temporary VM test directories were removed after validation. Regression tests are maintained in `src/update_revisions.rs` and `scripts/test_revision_manifest.py`.

Still required before distribution:

- Run CI on all supported architectures, including Windows x86 and macOS universal.
- Exercise an actual Windows MSI same-version revision upgrade. The release workflow downloads the current MSI revision for a hotfix and verifies the installed executable's revision after upgrading.
- Exercise actual DEB, RPM, and Arch updater installation and restart workflows using disposable installations.
- Verify the published GitHub manifest/download workflow, including an older released manager upgrading directly to the latest compatibility package.
- Configure the matching signing secret before publication.

The initial revision-2 test binaries above tested revision mechanics without the deferred Ubuntu tray and AppImage changes. The later live tests below included those changes in isolated test builds; they remain excluded from the revision-1 source.

## Follow-up live update testing

The user confirmed successful Windows x64 MSI and portable updates for both revision 1 → revision 2 and older 0.8.2 → revision 2 directly. Theme settings and preservation files survived the updates, and no repeat update was offered.

Live Linux portable transactions also passed on Ubuntu, Fedora, and Arch for both starting versions. The tests downloaded packages from the isolated `CryptoKey98/craft-apps-manager-update-tests` GitHub release, ran the real updater's check/prepare/launch/helper paths, verified an automatic graphical restart into revision 2, preserved light-theme/activity settings and a test data file, and confirmed that checking again returned no update.

Linux test builds added a test-only command-line entry point to invoke those updater functions; the updater implementations were unchanged. Linux revision 2 included the deferred Ubuntu/AppImage changes in the isolated test source. Those changes remain separate from the main development source. Test results are under `target/investigation/linux-live-revisions/results/`.

Native Ubuntu DEB, Fedora RPM, and Arch package transactions passed both upgrade paths through the real updater. All verified the graphical restart, revision 2, retained settings/data, and no repeat offer. The desktop test runners then restored the original production manager packages (Ubuntu 0.8.2, Fedora 0.7.0, and Arch 0.8.0-1). These verify successful installation transactions; interrupted installation and recovery are separate tests.

### Startup issue found during live testing

Fedora's GUI restart logs exposed an AccessKit worker panic (`there is no reactor running`). A separate single launch reproduced it, excluding duplicate test launch/cancellation as its cause. AccessKit's `async-io` executor and keyring shared zbus 4, but keyring enabled that dependency's Tokio backend. The backtrace showed AccessKit attempting `tokio::task::spawn_blocking` without a Tokio runtime.

The Linux keyring dependency now uses its `async-io` feature, preserving the encrypted Secret Service backend. A fresh graphical launch on Ubuntu, Fedora, and Arch then produced no background panic. A separate credential-store smoke binary successfully wrote, read, and deleted a unique dummy credential on all three systems; existing manager credentials were not altered. Windows formatting, all-target/all-feature Clippy, compilation, and the local release build passed after the change.

The public test release still contains the earlier immutable revision-2 packages used for the successful transaction tests. The dependency correction was validated with separate local binaries rather than silently replacing those published snapshots. Windows x86 and macOS runtime testing, failure/cancellation transactions, and production workflow/signing-secret setup remain outstanding.
