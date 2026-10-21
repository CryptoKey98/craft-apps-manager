# Signed manager updater operations

This describes the updater prepared for 1.0.0. It is not active in the published 0.9.0 manager. Production hosting still requires separate configuration and approval; these instructions do not authorize deployment or a release.

## Client and hosting boundaries

The publisher signs a bounded v2 catalog with the existing Ed25519 key. It describes supported releases, exact assets/hashes, relative payload layouts and eligible compatibility routes. The manager validates it independently, chooses only its own platform/architecture/installation type and executes built-in handlers. Metadata cannot supply shell commands, new trust keys or arbitrary installation paths.

The first server-aware build includes Windows ZIP, Linux ZIP/tar.gz, macOS ZIP/DMG and the existing native package methods. Filename changes and transitions between these supported transports go directly to the eligible release. A bridge is necessary only for missing future capabilities or required migration steps. One manager confirmation can authorize an eligible bridge chain; after every restart, the manager rechecks signed policy and the bounded continuation approval. Native chains can reuse one administrator approval through a temporary, verified helper. Initial installation is separate, and an installer may have its own prompts. No helper remains authorized for unrelated installations.

The separate Worker configuration is in `update-service/`. The static `/v2/catalog.json` path serves the signed envelope without dynamic selection. Static 404 handling prevents unknown paths from falling through to Worker code. The optional `/v2/check` route applies a per-location IP limit of 120 requests per minute before selection; its result must agree with the signed policy. Rejected dynamic requests still consume an invocation. Neither this limiter nor the free plan guarantees protection against distributed quota exhaustion. See [Cloudflare static routing](https://developers.cloudflare.com/workers/static-assets/routing/static-site-generation/) and [rate limits](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/).

No account secrets are shipped to clients. GitHub tokens are not attached to metadata requests. Downloads are restricted to the configured GitHub repository and approved HTTPS GitHub asset redirect hosts. Unknown formats/layouts remain ineligible; they do not turn a failed check into “latest version.”

## Local checks

Run both updater configurations and their serial regression tests from the project root:

```text
cargo fmt --all -- --check
cargo test --locked --all-targets -- --test-threads=1
cargo test --locked --all-targets --features service-updater -- --test-threads=1
cargo test --locked -p craft-updater-config -p craft-updater-service -- --test-threads=1
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --features service-updater -- -D warnings
python scripts/testing/test_updater_build.py
```

The build-mode check runs separately from Cargo tests to avoid nested build locks. Targeted updater suites are `tests/updater_*.rs`. Worker tests and configuration checks:

```text
cd update-service
npm ci --ignore-scripts --no-audit --no-fund
npm test
npm run typecheck
npm run dry-run
```

`dry-run` builds locally; it does not create or deploy a Worker. Dynamic processing is disabled in the checked-in configuration. Before enabling it, confirm limiter availability and its namespace on the free plan. Keep it disabled if those prerequisites are unmet.

`python scripts/testing/test-updater-transactions.py --out <local-evidence.json>` builds a separate gated fixture executable and exercises ZIP-only A → ZIP/tar.gz B → tar.gz C using isolated temporary installations, a test key and settings/library markers. It never modifies installed Craft apps or native packages. It exercises shared verification, routing, extraction and the existing helper replacement. Running it on Windows is Windows evidence for that shared machinery, not a Linux/macOS runtime test or proof of production v2 helper authorization. The fixture and its test key are excluded from default manager packaging. Native and real retained-release transactions require separately authorized disposable installations and a feed those binaries already trust.

## Activation prerequisites

Before a production build, choose the next application version, confirm the separate Worker name/address and review the actual platform transaction evidence. Configure these **build-time** values:

- `CRAFT_UPDATE_SOURCE`: `github` (the default for source builds) or `service` (explicit opt-in).
- `CRAFT_UPDATE_CATALOG_URL`: trusted HTTPS static catalog URL, required for a service build.
- `CRAFT_UPDATE_CATALOG_MIRROR`: optional GitHub HTTPS mirror of identical envelope bytes.
- `CRAFT_UPDATE_CHECK_URL`: optional dynamic `/v2/check` endpoint, after limiter verification.

For official GitHub release builds, configure the matching repository variables. The release workflow selects `service` and passes these endpoints to the compiler only when `CRAFT_UPDATE_V2_ENABLED` is `true`; otherwise it selects `github` and leaves the endpoints empty. Local builds can supply the build-time values directly. This wiring does not activate or deploy a service by itself.

A GitHub build uses the original release discovery, package selection and verification code. A service build never silently falls back to that protocol after an error; missing service configuration fails the build. Choosing GitHub explicitly when rebuilding does not erase previously accepted v2 trust records. Re-enabling service mode retains its anti-replay protection. Signed `metadata_origins` can introduce a new HTTPS catalog host for service builds without changing the trust key; keep the old origin serving current signed bytes for clients that have not received the migration.

### Production hosting status

The production Worker is hosted at `https://craft-apps-manager-updates.cryptokeyofficial.workers.dev`. Its signed standby catalog describes verified, already-published 0.9.0 packages and deliberately contains no update routes. It cannot publish 1.0.0 or offer a downgrade. Dynamic selection remains disabled.

Official publisher configuration is separate in root `updater.production.toml`; normal source builds continue using `updater.toml` and the GitHub updater. [Production catalog operations](../../update-service/catalog/README.md) describes bootstrap policy, the mirror, renewal gates and remaining activation steps. The checked-in renewal workflow must reach main before the daily schedule can run; a local signing check does not validate the GitHub-stored deployment token.

Before release activation, publish the real release packages, verify their remote bytes, and sign a catalog that passes the supported-client routing checks. Do not use placeholder hashes, the public fixture key, or test-repository packages. Preflight rejects the standby catalog for a new release tag.

### Source build choice

This is for people compiling the manager. Packaged releases use their publisher's build choice; there is no additional user-facing settings control. With the checked-in `source = "github"` configuration and no source override, enabling `--features service-updater` alone keeps the existing GitHub updater. Endpoint values alone do not activate the service.

To opt into the service on PowerShell (replace the example address with the publisher's real catalog address):

```powershell
$env:CRAFT_UPDATE_SOURCE = 'service'
$env:CRAFT_UPDATE_CATALOG_URL = 'https://updates.example.com/v2/catalog.json'
cargo build --release --locked --features service-updater --bin craft-apps-manager
```

On Linux/macOS:

```sh
CRAFT_UPDATE_SOURCE=service \
CRAFT_UPDATE_CATALOG_URL=https://updates.example.com/v2/catalog.json \
cargo build --release --locked --features service-updater --bin craft-apps-manager
```

To switch back, rebuild with `CRAFT_UPDATE_SOURCE=github`; endpoints do not need to be removed. This changes the newly compiled binary, not an already-running manager. No API token or private signing key is needed by the client. The catalog must match the binary's pinned public key, repository and channel. The isolated test Worker uses a different public test key and must not be used by ordinary production-key builds.

The choice is compiled into each binary. Replacing a custom build with an official update adopts the official build's updater choice. To retain a custom choice across releases, rebuild subsequent versions with the same option, or publish matching packages in a fork with `CRAFT_MANAGER_REPOSITORY` set to that fork. GitHub-only packages still require the original compatible release assets; v2-only transport/name migrations apply to service-enabled builds.

Source/publisher configuration now lives in the commented root `updater.toml`.
Ordinary source builds use GitHub and do not include `craft-updater-service`.
Enabling Cargo's `service-updater` feature includes the library; selecting
`source = "service"` (or CRAFT_UPDATE_SOURCE=service) activates it. Selecting
service without the feature fails the build. Official releases coordinate both
choices explicitly behind their existing activation gate; forks do not inherit
service activation merely by building release mode.

The required `craft-updater-config` crate handles configuration only. Its standalone
export tool works without compiling the GUI or service dependency:

```sh
cargo run --locked -p craft-updater-config --bin updater-config-tool -- export \
  --project-root . --view build --target-os linux
```

Precedence is defaults, selected TOML, explicit supported environment overrides,
then named CLI overrides. An empty explicit endpoint clears the file value;
unknown fields, invalid paths, unsupported formats and missing service settings
fail. Public-key/profile/catalog paths are relative to the selected TOML and
must stay within the project. Never put private keys or account tokens in TOML.
Installing an official binary adopts that binary's embedded configuration.
The future setup/key-generation script is not included in this change.

Keep `[publishing].enabled = false` until activation is approved. Activation
requires service endpoints, supported-client `profiles_file`, `next_catalog_file`,
existing `mirror_tag` and matching repository/channel/public key. Exact release
assets, hashes and bridge routes remain signed catalog data. Legacy downloads
remain enabled by default. Linux tar.gz packaging currently reuses the verified
ZIP output and requires ZIP in its configured package list.

`asset_name_pattern` controls immutable service names independently of client
selection. It must retain distinct revision identities and cannot overwrite
signed bridge assets. Only handlers already compiled into the manager are usable;
changing the package list cannot add Flatpak or other unimplemented formats.

Worker settings are generated from the same resolver, not a second editable
Wrangler file. `npm run dry-run` refreshes ignored `wrangler.generated.json` and
validates it locally. The separate `wrangler.test.jsonc` remains isolated.
The renewal workflow checks daily at 08:17 UTC and only signs near expiry (or
for an explicitly requested release policy). Configuration does not authorize
an upload, production activation or deployment.

Set the protected repository variables `CRAFT_UPDATE_V2_ENABLED` and `CRAFT_UPDATE_V2_DEPLOY_ENABLED` only after approval. Configure the `manager-update-production` environment with required reviewers, scoped Cloudflare credentials, account ID and an existing `[publishing].mirror_tag` in updater.toml. Keep signing access separate from deployment. Retain an offline key backup. The Worker receives only the public key and signed catalog, never the private signing key.

## Isolated test Worker

With explicit owner approval, a separate test Worker was deployed on 2026-10-16:

- Base: `https://craft-apps-manager-updates-test.cryptokeyofficial.workers.dev`
- Human-readable status page: `/index.html`. The bare `/` intentionally returns 404 because exact-path HTML handling is retained.
- Static signed catalog: `/v2/catalog.json`; optional dynamic selection: `/v2/check`.
- Config: `update-service/wrangler.test.jsonc`, with separate assets, limiter namespace, no custom routes and no preview URLs.
- Scope: repository `owner/manager`, channel `test`, public fixture signing key. Production managers reject that key. No production signing key is read or uploaded.

The initial service hosted metadata/routing fixtures only. It now also serves isolated Windows fixture packages for the transaction checks below; these are **not application releases**. Do not point a normal manager build at this endpoint or weaken its production trust. It does not change public update metadata, GitHub releases or production activation variables.

From `update-service/`, regenerate its seven-day test catalog and validate before a separately authorized redeployment:

```text
node --experimental-strip-types scripts/prepare-test-deployment.ts
npm test
npm run typecheck
node node_modules/wrangler/bin/wrangler.js deploy --config wrangler.test.jsonc --dry-run --outdir .wrangler/test-dry-run
node node_modules/wrangler/bin/wrangler.js deploy --config wrangler.test.jsonc
```

Generated test assets/evidence stay under ignored `.wrangler/`. Test catalog renewal is manual; there is no production cron or deployment automation. The deterministic fixture seed is public test material and must never become a production signing key.

The opt-in Rust test `isolated_worker_signed_catalog_smoke` uses the actual metadata transport, signature verifier, check coordinator and routing with separate temporary roots. Set `CRAFT_TEST_UPDATE_URL` to the test base URL ending in `/`, then run from the repository root:

```text
cargo test --locked --features service-updater --test updater_live_service -- --ignored --test-threads=1 --nocapture
```

It checks both static and dynamic direct/bridge/current outcomes and rejection by the production public key. It never downloads packages, launches installers or changes manager settings. Local tests check the rate-limiter behavior; the live smoke check deliberately does not exhaust quotas. This metadata-only check does not establish that package replacement, native installers or older-manager migration work; qualify those with separate transaction tests.

### Live Windows fixture transactions

`scripts/testing/test-worker-updates.py` builds the feature-gated `update-worker-fixture` executable at isolated A/B/C versions and packages B/C for this test Worker. B performs a fixture-data schema migration required by C. It exercises actual B→C direct and A→B→C bridge replacements/restarts, with fresh checking and explicit fixture approval at each hop.

Run from the repository root, redeploying **only** the test configuration:

```text
python scripts/testing/test_worker_update_harness.py
python scripts/testing/test-worker-updates.py prepare
```

Then from `update-service/`:

```text
node node_modules/wrangler/bin/wrangler.js deploy --config wrangler.test.jsonc --dry-run --outdir .wrangler/test-dry-run
node node_modules/wrangler/bin/wrangler.js deploy --config wrangler.test.jsonc
```

Finally from the repository root:

```text
python scripts/testing/test-worker-updates.py run
```

Generated executables, packages, isolated installations and `evidence.json` stay under ignored `update-service/.wrangler/transactions/`. The harness checks executable hashes/identities, child exit/restart receipts, byte-for-byte preservation of both real settings-file roles, selected and alternate app-folder records, app executable/project files, portable markers and an external installation. It also checks refusal without approval, mismatched helper/source hashes and tampered package bytes without changing the installed fixture.

This test executable uses shared production metadata transport, signature/routing/trust, archive staging, build-identity and commit-authorization functions plus the existing executable replacement helper. Its fixture-only adapter retrieves signed hash/size/asset identities from the same test Worker's `/artifacts/` path. Production URL validation/downloads remain restricted to GitHub. Consequently, successful fixture transactions do **not** prove the production GitHub download path, the production-v2 authorization wrapper dispatch, the full GUI startup, retained old managers, native installers, or Linux/macOS runtime behavior. Qualify these separately using actual manager packages and retained client builds. Production binaries never acquire the public fixture key or test endpoint.

## Publishing and renewal

1. Build/test artifacts; preserve current legacy filenames, v1 signatures and native compatibility versions during migration. First install the server-aware release through that old compatible path. Managers that have never installed it still only check GitHub latest.
2. When v2 publishing is enabled, snapshot immutable v2 asset copies **before** the v1 renaming pass. Publish package bytes before advertising them. Do not clobber a v2 URL with a new revision or delete a required historical bridge. Native package version remains separate from filename/display version.
3. Validate the explicit candidate catalog and every supported client profile through `update-catalog-tool`, which uses the actual Rust router. All supported profiles must reach the newest target. Unknown future capabilities must have retained compatible bridges.
4. `update-catalog.py verify-assets` verifies remote sizes/digests and emits a short-lived receipt bound to candidate bytes. `build` requires that receipt, the authenticated previous catalog and a strictly higher sequence. First activation uses an explicit `--initial` instead of pretending a previous catalog exists. The signing key must match the application's pinned public key.
5. Deploy identical envelope bytes atomically with Worker assets and mirror them on GitHub. Verify the public response. The initial activation is an explicitly approved manual operation; the renewal workflow requires an existing authoritative catalog and cannot bootstrap itself.

`renew-update-catalog.yml` is disabled unless the activation variable is true. Its sign job authenticates the previous catalog before inspecting its package instructions; the separate protected deploy job requires the deployment variable as well. The workflow checks daily and renews only when the configured expiry threshold is reached. Renewal advances sequence and extends validity to the configured duration (30 days by default), even without binary changes. Release publishing calls the same serialized workflow with next-release policy. Enable workflow-failure notifications for maintainers; a failed renewal must be investigated before expiry. There is no indefinite expired-catalog bypass.

## Failure, recovery and retention

Transport failure can use static/mirror metadata; signature, scope, expiry or sequence failures cannot downgrade trust. A successful check clears bounded backoff. Manual requests share a 60-second cache; periodic requests use existing preferences. Per-data-root check locks and canonical-target installation locks coordinate separate processes. Hash/size, application identity, platform, native ownership and helper-plan checks remain mandatory. Staged archive payloads are rebuilt from authenticated artifacts in the v2 helper before commit.

Bridge restarts bypass the successful-result cache so they fetch current policy; failure/429 backoff remains in force. Immediately before replacement or handing control to a native installer, the updater rechecks expiry, signed selection and the highest accepted catalog sequence. A trust guard serializes that decision with metadata acceptance through the transaction. Once an external installer owns the operation, it may need to finish; a later policy change is not a promise of cancellation or rollback. Linux native success also requires the installed executable's actual version, revision and architecture to match the signed identity.

Immutable asset uploads are preflighted as a group. Already-published identical bytes are left alone, different bytes stop the upload, and missing assets are uploaded without clobber. Mutable legacy downloads use a separate path. This protects existing signed catalogs even if a configured naming pattern accidentally reuses an older filename.

Keep helper/staging failure records until ownership and recovery needs are established. A failed native installer is not assumed rollback-safe. A DMG detach failure retains its exact cleanup record; never sweep arbitrary mounted volumes. Cancellation before commit leaves the installed program untouched; installation already handed to a native package manager may need to finish before the manager can report a verified result.

Correct a bad catalog by signing a **higher** sequence. Do not replay an earlier deployment/catalog as a rollback. Reverting bad application code also needs a new version/revision. Retain necessary bridge assets in historical public releases and preserve the verification path they require. Do not draft/delete them as cleanup. Retiring legacy downloads is an explicit owner decision and may require a manual update for pre-server-aware managers that missed the transition.

This is a custom signed-catalog protocol, not full TUF. Sole-key loss/compromise, missing bridges or an unsupported OS can still require manual recovery. Threshold trust-root rotation and actual Flatpak packaging remain separate projects.

### Packaging configuration checks

Packaging compares an embedded fingerprint of the selected runtime updater configuration,
including endpoints, timeouts, key, scope and revision. Changing those values requires
rebuilding the executable. Local config paths and publisher-only choices do not enter
that fingerprint. It is a consistency check; signed catalogs and package hashes remain
the security boundary.

macOS packaging reads a dedicated identity section from every Mach-O slice before
signing, so the other CPU architecture cannot silently carry an older configuration.
Service-activated releases validate publisher readiness, signing-key agreement and
policy/profile scope before building or uploading packages. Format selections also gate
MSI upgrade smoke tests and the Arch package job.
