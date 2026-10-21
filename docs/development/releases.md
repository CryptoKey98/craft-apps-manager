# Release preparation

For maintainers publishing the manager. Building a local executable is separate; see [Building and testing](building.md). Local validation does not publish anything.

## Prepare the version and notes

`Cargo.toml` is the application version source. The release tool coordinates its version with Cargo.lock, the README caption, and the changelog.

From the repository root, replace the example version with the version you are preparing:

```sh
cargo run --locked --features release-tools --bin release-tool -- bump 1.0.0 --dry-run
cargo run --locked --features release-tools --bin release-tool -- bump 1.0.0
```

Run the bump only for a version change, not to republish an existing version. Replace the new changelog placeholder with concise user-facing changes. Update What's New and review package/platform scope. Preserve older changelog entries.

Validate before opening a PR:

```sh
cargo run --locked --features release-tools --bin release-tool -- check
cargo test --locked --features release-tools --bin release-tool
python scripts/testing/test_validate_release.py
```

Run the relevant [application checks](building.md#tests-and-static-checks) too. Use `python3` on Linux/macOS. Stable versions have three numeric parts and must fit MSI limits: major/minor up to 255, patch up to 65535.

Merge the reviewed changes only after required CI passes. Record the exact main commit that will be built.

## Prepare service update policy

For a service-enabled release, update the candidate policy and client profiles referenced by `updater.production.toml` before running Release. The workflow does not invent release routes or new package handlers. Candidate entries must describe the intended release, platform, architecture, package, layout, and any required bridge route.

Release-mode catalog publication replaces new candidate entries' draft/build hashes with the actual published asset size and SHA-256 digest from GitHub's authenticated inventory. It independently downloads and hashes those packages before signing. Existing signed package identities are not rewritten. See [Updater operations](updater-operations.md#publishing-and-renewal) for trust, renewal, and retention requirements.

## Publish through GitHub Actions

Normal releases **publish automatically** after their package jobs succeed. You do not need to create a draft first.

For a manual run, open **Actions → Release → Run workflow** and choose:

| Input | Full-release value |
| --- | --- |
| Branch | `main` |
| `tag` | `v` followed by the version, for example `v1.0.0` |
| `revision` | `1` |
| `platform` | `all` |
| `publish_baseline` | Enabled (the default) |
| `prepare_draft` | Disabled (the default) |
| `source_ref` | The full 40-character SHA of the merged main commit |

The workflow builds from that exact source commit. It validates notes and configuration, builds the selected packages, creates checksums and signed v1 metadata, and publishes the release targeting that commit. Tag-triggered runs also publish normally; a manual run does not need a pre-created Git tag. Starting publication should be an intentional maintainer action.

With service publication enabled, the release then calls **Renew signed manager update catalog** in `release` mode. That verifies the published packages, signs the next catalog sequence, deploys the Worker, updates the stable GitHub mirror, and compares the public responses. A green package job alone does not prove this last stage succeeded.

The workflow refuses to overwrite a published baseline. Do not rerun a successful full release just to repair its catalog, and do not replace published package bytes referenced by signed policy.

### Optional preview

Only when you intentionally want an unpublished preview, select `revision=1`, `platform=all`, `publish_baseline=false`, and `prepare_draft=true`. This creates a draft automatically if absent and cannot deploy update offers. Preflight still checks source configuration, trust, and profiles; final-package verification happens after packages exist. Existing immutable package names cannot be overwritten with different bytes.

## Publish a platform revision

A revision is a platform-specific rebuild of the same application version. Revision 1 is hidden in the display; 2 and later appear as `Rev. N`.

Use the Release workflow with the existing version tag, an advancing revision, the affected platform, and the exact source commit. Keep `prepare_draft` off. The baseline release must already be public. Native package versions and the candidate catalog must describe the new build.

Service-enabled revisions use distinct `-Rev-N` package names. Both signed v1 and v2 metadata can reference them; baseline URLs stay unchanged. Older manifest-aware clients can select the signed revision asset. Pre-manifest clients can first reach the compatible baseline, then update through the newer updater. GitHub-only publishing retains its separate compatibility-refresh behavior.

Retain compatible downloads while older clients still need them. A signed catalog can change filenames or choose another already-supported format for service clients; it cannot teach an old binary a new handler. Keep any package required by a supported bridge route.

## Recover a catalog-only failure

If package publication succeeded but catalog signing or deployment failed:

1. Inspect the failed step and verify that the release is already public.
2. Correct the policy or publisher issue; keep existing package files intact.
3. Merge the tested fix, then open **Actions → Renew signed manager update catalog → Run workflow**.
4. Choose `main`, `mode=release`, and leave `force_renew` off.
5. Check both signing and deployment, plus the public Worker and mirror.

A failed-job rerun uses the original run's source and workflow. If the fix is in a newer commit, start a new catalog workflow from that corrected source instead. This does not rebuild packages or create another application release.

For ordinary metadata expiry renewal without changing offers, use `mode=renew`. Scheduled renewal already runs daily; see [Renewal](updater-operations.md#publishing-and-renewal).

## Confirm publication

- The intended release is public and marked latest where appropriate.
- Expected packages, `SHA256SUMS.txt`, and `updates-v1.json` exist; baseline packages have no duplicate `update` copies.
- Legacy metadata and the v2 catalog verify with the pinned public key.
- Public package bodies match signed sizes and hashes.
- Worker and stable mirror return identical valid catalog bytes; supported client profiles get the intended current/direct/bridge result.
- Actual update transactions are qualified on the affected platforms when behavior changes. Metadata checks alone are not installer tests.

The release tool itself never commits, pushes, tags, uploads, or publishes.

[Documentation index](../README.md)
