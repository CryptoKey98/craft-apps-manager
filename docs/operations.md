# Operations and Activity

`src/operations.rs` is the authoritative lifecycle store. Operation records have unique identifiers, owner processes, resource identities, optional parents, stages, measured progress, timestamps, cancellation status and result details. Records live under the library's `runtime/operations` directory so the manager, builder and elevated workers can coordinate across process boundaries.

Operations transition from Queued to Running and then Completed, Failed or Cancelled. Cancellation Requested is a request, not confirmation that execution stopped. Terminal records reject delayed progress and duplicate completion. Stale owners are reported as Interrupted rather than successful. Execution locks and worker restrictions remain in `src/jobs.rs`; tracking multiple records does not permit conflicting installations.

Bulk operations have a parent and independently tracked children. Results from completed children survive a later failure. Pending or interrupted children are not reported as successfully installed. Windows batch approval retains its verified manifest, elevated worker, progress protocol and cancellation signaling. Linux uses a separate temporary approved-package worker; package transactions finish safely before remaining work is cancelled. Portable operations run without elevation.

`src/ui/operations_view.rs` renders the compact Activity panel from these records. Existing progress views observe the same operation state. The badge counts unread root activity during the current manager session; opening Activity marks it read. Preparation of an install preview is not an installation. Expanded details use recorded outcomes and paths, rather than claiming unverified success or rollback.

The separate builder uses the shared library's operation records for source downloads, extraction, compilation, tool setup and cancellation. Headless checks keep their existing execution and notification behavior; they do not wait for interactive installation choices. Completed history is bounded. Operations are not automatically retried or destructively resumed after a crash.

## Validation

Local release-profile validation passed 169 Windows tests and 143 Ubuntu tests; one real Windows MSI test was intentionally ignored. Rust formatting and strict Clippy passed on Windows and Ubuntu. The final dark Activity surface change was compiled in a Windows release build. CI provides Windows x64/x86, Linux x64/x86 and macOS checks.

Manual validation remains necessary for native installer transactions, actual UAC/PolicyKit batch approval, mixed batches and cancellation during a package transaction. Arch and Fedora batch execution and macOS runtime behavior have not been verified locally. Custom EXE installers may register incompletely or spawn other processes; automatic detection cannot guarantee support for every publisher. Existing recovery and backups remain available, but this redesign does not provide universal automatic rollback.
