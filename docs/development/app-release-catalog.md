# App-release catalog

The app-release catalog provides release versions, downloadable packages, sizes, checksums, verified Windows installer compatibility, and official discovery dates. It is separate from the Manager's own update service.

## Client behavior

- Supported built-in and discovered Craft apps can use the configured catalog. Custom GitHub apps use the GitHub API directly.
- The client verifies the signed root and each requested app object before using them. Catalog data cannot supply executable commands or bypass installation checks.
- Missing app entries, unavailable hosting, or unusable metadata fall back to the existing GitHub request path automatically.
- Verified metadata stays in memory and a bounded compressed disk cache. Normal revalidation is 60 seconds; failures use increasing retry intervals. Clearing GitHub Cache clears this cache too.
- Newer GitHub responses update one shared release view, including latest, version history and selected-tag download details. Reusing cached API data does not renew its observation time. Matching catalog data is adopted automatically; a later successful signed scan can supersede an API observation, including a withdrawn release. Expired or evicted conflicting API bodies require revalidation rather than silently restoring a known older catalog result.
- Automatic/startup checks and normal browsing use each repository's successful signed observation for up to 24 hours. Downloading the feed again does not renew that observation. Missing, invalid or expired data falls back per app/query to GitHub. An unavailable server does not erase a previously verified eligible copy.
- An explicit **Check for updates** uses GitHub directly, sharing a successful response for 60 seconds and coalescing concurrent requests. A newly found release appears immediately without waiting for catalog publication. A failed check retains the last known update indication while reporting the failure; it does not claim the app is up to date. Local version/format/architecture changes reuse loaded information.
- Installation-time revalidation uses GitHub's short Update cache separately from catalog browsing. Signature, repository, download size/hash, compatibility, scope and recovery checks remain in force.
- The local catalog cache is disposable and versioned separately from user settings. Older cache records are migrated or safely refetched; installed apps, preferences and discovery dismissals are unaffected. API release bodies share the existing bounded compressed store; they do not create a file per observation.
- Discovery dates need only the signed root, without downloading per-app objects or installers. Installed app data and badge dismissals are stored separately from disposable cache.

The top-level `app_release_catalog` option in the selected updater TOML controls the compiled client choice, independently of the Manager self-updater's `source`. It defaults to `false` in source builds; `updater.production.toml` explicitly sets it to `true` for official builds. Opted-in builds use the public endpoint and trusted keys in `assets/app-release-catalog/source.json`; that descriptor's `enabled` flag must also be true. Merely enabling the `service-updater` Cargo feature does not enable the app catalog. The `app-catalog-test-source` feature accepts the compile-time `CRAFT_APP_CATALOG_TEST_SOURCE_JSON` override for isolated testing. Never put a private signing key in either source descriptor.

### When clients pick up a publication

| Action | Release-information path |
| --- | --- |
| Open Install, open a version list, or request a selected release's details | Use eligible shared metadata; revalidate the catalog when its refresh interval allows. Loaded option changes reuse the same release data. |
| Enabled startup or periodic app update check | Prefer the catalog, refreshing it when due; fall back to GitHub for missing or expired app information. |
| Refresh the discovered app list | Refresh the signed root for official admission dates, without downloading installers. Discovery itself still uses GitHub. |
| Click **Check for updates** manually | Query GitHub directly through the shared 60-second Update cache. Feed the result into the version list immediately; do not wait for a catalog publication. |
| Validate a download before installing | Revalidate with GitHub's short Update cache; retain all download and installation checks. |
| Clear GitHub Cache | Remove saved API and catalog data. The next relevant request fetches data again; clearing alone does not start a download. |

There is no separate catalog polling timer while the Manager sits idle. The next release request or enabled automatic check can pick up a newer publication. A restart can reuse a recent verified disk copy; it does not force a download. Version-list browsing can return eligible cached data while the refresh runs in the background.

The three clocks serve different purposes:

- **60 seconds:** the normal minimum interval between client catalog refresh attempts. Failures back off to 5, 15, then 60 minutes. Manual GitHub checks have a separate 60-second successful-response cache.
- **Publication cadence:** the production VPS checks every 10 minutes and requests a refresh when it detects changes or the last successful publication is at least 12 hours old. GitHub Actions queues and publishes those requests. Maintainers can also dispatch a refresh manually.
- **24 hours:** the maximum age of each app's last successful upstream observation for catalog-backed browsing and automatic checks. A successful check can renew that app's timestamp even when its releases are unchanged. Republishing the root, downloading it again, or failing to check an app does not renew that app's timestamp.

For example, a publication containing a successful FilmCraft scan from 25 hours ago cannot supply its release history to a client. That query falls back to GitHub even if the signed root was published just now. This rule concerns app metadata, not the separate Manager self-update catalog's validity period.

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

The workflow supports manual validation and external dispatch. The production scheduler is maintained separately from this repository and dispatches from `main` with the publication input enabled. Older Managers retain their 30-minute freshness threshold and may use the GitHub API more often when publication is less frequent.

GitHub can queue externally triggered runs. Actions independently collects, admits, inspects, signs and verifies. Check the successful run and verified hosted feed before considering a publication complete. Each collection run inspects at most eight previously unverified MSI files, without installing or executing them.

Collection has read-only repository access and no signing secret. Publication runs only from `main`, behind the `CRAFT_APP_CATALOG_ENABLED=true` repository variable and the `app-release-catalog-production` environment. Store the dedicated private key as that environment's `APP_RELEASE_CATALOG_SIGNING_KEY` secret. The corresponding public key belongs in `assets/app-release-catalog/source.json`. When more than one public key is trusted during rotation, select the active key with `CRAFT_APP_CATALOG_KEY_ID`.

Before activation, run the workflow manually with publication unchecked and review its candidate. Configure the protected signing environment, then run an explicitly enabled publication and verify the hosted signature and app objects. Enable the client source only after that verification, and prepare its bundled baseline from the verified publication. Keep the source disabled while hosting is unconfigured; the existing GitHub API path remains available.

Generate the shipping baseline from a verified local checkout of the data branch with `python scripts/publishing/app_release_catalog.py baseline --tool target/release/app-release-catalog-tool.exe --feed <catalog-checkout> --out assets/app-release-catalog/baseline.json` (omit `.exe` on Unix). This verifies signatures and every referenced app object before writing. A source build using different trust keys ignores an unrelated bundled baseline and keeps using its own configured feed or GitHub fallback.

For an explicit read-only client smoke test, set `CRAFT_CATALOG_SMOKE_SOURCE` to the absolute path of the source descriptor, then run `cargo test --locked --features app-catalog-tools --lib hosted_ -- --ignored --nocapture --test-threads=1`. Use the normal GitHub updater configuration, or also enable `service-updater` when selecting the official updater configuration. These tests check version history, the latest release, verified compatibility, and SolidCraft/TextureCraft enrollment without GitHub API fallback or installer downloads. Their temporary cache is removed afterward.

The compiled publisher is reused only for the exact workflow source revision, compiler version, and Windows runner architecture. A new source commit or compiler version requires a build. The executable cache holds only the binary and its identity/checksum receipt; a missing or invalid entry causes a rebuild. Publication tests still run on every refresh, and the artifact hash is checked again on the signing runner. The publisher refuses a stale branch checkpoint, changed immutable bytes, invalid signatures, or a mismatched signing key. App objects are pushed before the signed root. Publication uses ordinary fast-forward pushes, never force-pushes, and never creates an application release.

Current and previous signed roots remain available. Superseded objects and historical roots are retained for at least one day, and objects referenced by retained roots are protected. Cleanup only removes tracked, hash-verified objects with recorded ownership; unknown files are preserved. Interrupted object staging is recorded for later safe cleanup. Temporary workflow artifacts expire after one day.

Internal testing must use a separate feed, signing key and data folder. Remove temporary workflows, branches, scheduler units, credentials and local builds when a test is finished; retain regression tests with the implementation. Local preparation does not change the live public feed, configure GitHub secrets, replace a VPS schedule, or publish metadata.
