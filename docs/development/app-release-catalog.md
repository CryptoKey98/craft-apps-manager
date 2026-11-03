# App-release catalog

The app-release catalog provides release versions, downloadable packages, sizes, checksums, verified Windows installer compatibility, and official discovery dates. It is separate from the Manager's own update service.

## Client behavior

- Supported built-in and discovered Craft apps can use the configured catalog. Custom GitHub apps use the GitHub API directly.
- The client verifies the signed root and each requested app object before using them. Catalog data cannot supply executable commands or bypass installation checks.
- Missing app entries, unavailable hosting, or unusable metadata fall back to the existing GitHub request path automatically.
- Verified metadata stays in memory and a bounded compressed disk cache. Normal revalidation is 60 seconds; failures use increasing retry intervals. Clearing GitHub Cache clears this cache too.
- Newer GitHub responses take precedence when their release details differ from the catalog. Reusing cached API data does not extend that precedence, and matching catalog data is adopted automatically. A later successful signed scan supersedes older API observations.
- Update checks trust a repository's catalog observation for up to 30 minutes. Refreshing the feed does not itself recheck GitHub. A release published after the publisher's last scan may therefore appear after the next scan; older observations fall back to GitHub automatically. This applies to manual and automatic checks for catalog-backed apps.
- Discovery dates need only the signed root, without downloading per-app objects or installers. Installed app data and badge dismissals are stored separately from disposable cache.

`assets/app-release-catalog/source.json` controls the public feed and trusted keys used by normal builds. Disable its `enabled` flag to use GitHub API lookups without the catalog. The `app-catalog-test-source` feature accepts the compile-time `CRAFT_APP_CATALOG_TEST_SOURCE_JSON` override for isolated testing. Never put a private signing key in either source descriptor.

## Discovery and announcement dates

The Manager and publisher share repository and desktop-release admission checks. Storytold repositories are scanned automatically; explicitly supported external sources are listed in `src/catalog/supported.rs`. SolidCraft uses its pinned `tangency-labs/solidcraft` origin. Unsupported or unrelated repositories are not enrolled merely because they have a similar name.

The publisher records the first successful official admission in signed metadata and preserves that date through subsequent releases, inspection batches, and failed scans. The New badge expires 14 days after this date or when the user views the app, whichever comes first. A fresh install does not start a new period.

`assets/app-release-catalog/admissions.json` supplies shared offline dates for deliberately announced additions. Null entries identify established apps with no known historical admission date; they are not announced again during migration. The file is not a replacement for the publisher's persistent signed history. An API-discovered app without an authoritative date remains usable while waiting for the publisher's first enrollment.

New app ordering is adopted into preferences once. Rendering uses the saved sidebar and Home orders, so automatic promotion does not override later manual sorting.

## Publisher workflow

Build the separate maintainer tool with:

```sh
cargo build --release --locked --features app-catalog-tools --bin app-release-catalog-tool
```

The tool is not part of downloaded Manager packages. The normal publishing sequence is:

1. `enroll`: extend the supported repository list using shared admission checks.
2. `discover`: collect release metadata using the previous checkpoint when available.
3. `aggregate`: combine successful metadata and exact-file compatibility evidence, preserving earlier official dates and unavailable repositories' last verified information.
4. `sign` and `verify`: sign with the feed's dedicated key and verify all referenced objects before publishing.
5. Inspect previously unverified MSI packages in bounded batches and publish verified checkpoints. Unchanged file evidence is reused; installers are never executed for inspection.

Retain the previous publication and discovery checkpoint between runs. Losing publisher history can lose an app's original admission date; do not replace a feed with an empty publication to clean its cache. Keep signing keys outside the repository, and use separate trust keys and hosting for tests.

## Public hosting and activation

The public feed uses the separate `app-release-catalog` branch of the same repository. Application code and the publisher workflow stay on `main`; the data branch contains signed `v1/index.json`, immutable `v1/apps/<sha256>.json`, retained `v1/roots/<sequence>.json`, and incremental publisher checkpoints. Installer packages and compiled publisher tools are not committed to that branch.

The workflow provides a daily fallback at 08:37 UTC and supports manual validation. Regular refreshes can use an external scheduler to call `workflow_dispatch` on `main` with `publish=true`; the deployed scheduler checks every 30 minutes and skips a dispatch when a run is already active. GitHub can delay its own scheduled runs or queue externally triggered jobs. Runs are serialized; unchanged release pages and package inspection results are reused. Each collection run inspects at most eight previously unverified MSI files, without installing or executing them.

Collection has read-only repository access and no signing secret. Publication runs only from `main`, behind the `CRAFT_APP_CATALOG_ENABLED=true` repository variable and the `app-release-catalog-production` environment. Store the dedicated private key as that environment's `APP_RELEASE_CATALOG_SIGNING_KEY` secret. The corresponding public key belongs in `assets/app-release-catalog/source.json`. When more than one public key is trusted during rotation, select the active key with `CRAFT_APP_CATALOG_KEY_ID`.

Before activation, run the workflow manually with publication unchecked and review its candidate. Configure the protected signing environment, then run an explicitly enabled publication and verify the hosted signature and app objects. Enable the client source only after that verification, and prepare its bundled baseline from the verified publication. Keep the source disabled while hosting is unconfigured; the existing GitHub API path remains available.

Generate the shipping baseline from a verified local checkout of the data branch with `python scripts/publishing/app_release_catalog.py baseline --tool target/release/app-release-catalog-tool.exe --feed <catalog-checkout> --out assets/app-release-catalog/baseline.json` (omit `.exe` on Unix). This verifies signatures and every referenced app object before writing. A source build using different trust keys ignores an unrelated bundled baseline and keeps using its own configured feed or GitHub fallback.

For an explicit read-only client smoke test, set `CRAFT_CATALOG_SMOKE_SOURCE` to the absolute path of the source descriptor, then run `cargo test --locked --features app-catalog-tools --lib hosted_ -- --ignored --nocapture --test-threads=1`. Use the normal GitHub updater configuration, or also enable `service-updater` when selecting the official updater configuration. These tests check version history, the latest release, verified compatibility, and SolidCraft/TextureCraft enrollment without GitHub API fallback or installer downloads. Their temporary cache is removed afterward.

The compiled publisher is reused only for the exact workflow source revision, compiler version, and Windows runner architecture. A new source commit or compiler version requires a build. The executable cache holds only the binary and its identity/checksum receipt; a missing or invalid entry causes a rebuild. Publication tests still run on every refresh, and the artifact hash is checked again on the signing runner. The publisher refuses a stale branch checkpoint, changed immutable bytes, invalid signatures, or a mismatched signing key. App objects are pushed before the signed root. Publication uses ordinary fast-forward pushes, never force-pushes, and never creates an application release.

Current and previous signed roots remain available. Superseded objects and historical roots are retained for at least 30 days, and objects referenced by retained roots are protected. Cleanup only removes tracked, hash-verified objects with recorded ownership; unknown files are preserved. Interrupted object staging is recorded for later safe cleanup. Temporary workflow artifacts expire after one day.

The internal testing feed remains separate, with a different signing key and endpoint. Local preparation does not create the public branch, configure GitHub secrets, activate the schedule, or publish metadata.
