# Production catalog inputs

For maintainers editing update policy. Start with [Release preparation](../../docs/development/releases.md) for publication and recovery, or [Updater operations](../../docs/development/updater-operations.md) for protocol and trust rules.

## Files and responsibilities

| File | Purpose |
| --- | --- |
| `standby.json` | Bootstrap records for published 0.9.0 packages, with no routes; not the current production policy |
| `release-1.0.0.json` | Candidate release policy: package identities, supported layouts, and update routes |
| `supported-clients.json` | Client profiles checked with the actual Rust router before signing |
| `../../updater.production.toml` | Official build, publication, hosting, mirror, and renewal configuration |

The live signed catalog is authoritative. The candidate is an input template, not an envelope to deploy directly. Release-mode publication binds new entries to the actual published GitHub assets, independently downloads and hashes the files, and signs a higher sequence using the pinned key. Old draft hashes cannot substitute for the published build. Existing signed identities are not refreshed.

Never place private keys, API tokens, or test-package references in these files. The public verification key is safe to distribute; signing material remains private.

## Hosting and mirror

The production Worker serves the signed catalog at `/v2/catalog.json`; dynamic `/v2/check` is disabled in the checked-in production configuration. Package downloads remain GitHub release assets.

The stable mirror is `updates-v2.json` on the **v0.9.0 release**. It serves the same envelope as the Worker and is not copied onto every release. Keep that release available while clients reference it. Either endpoint must pass signature, scope, expiry, and sequence checks.

## Renewal

Production defaults are 30 days of catalog validity and renewal when seven days or less remain. GitHub Actions checks daily at 08:17 UTC (01:17 Arizona time). Renewal copies the verified live policy, verifies package bodies, increments the sequence, signs, deploys identical bytes to both endpoints, and compares their public responses. Routine renewal needs no manual action while credentials and dependencies remain healthy.

`CRAFT_UPDATE_CATALOG_RENEW_ENABLED` or `CRAFT_UPDATE_V2_ENABLED` enables signing; `CRAFT_UPDATE_V2_DEPLOY_ENABLED` enables deployment. The signing key is a repository secret. `CLOUDFLARE_API_TOKEN` belongs in the `manager-update-production` environment and needs scoped Workers Scripts Edit and Account Settings Read access for the deployment account. Never log either credential. Monitor workflow failures and token expiration.

## Shared packages and revisions

Revision 1 uses one package per platform/format for manual downloads and both signed updater protocols. There are no duplicate `update` copies. Later service-enabled revisions use distinct `-Rev-N` asset names, with signed metadata pointing to them. Baseline URLs remain unchanged.

Keep compatible downloads for older managers and packages required by bridge routes. Previously signed URLs cannot be overwritten with different bytes. Native package versions are separate from the filename and displayed revision.

## Recover publication

If packages are already public and only catalog publication failed, fix and validate the publisher/policy, then run **Renew signed manager update catalog** from the corrected source with `mode=release`. Do not rebuild successful packages or replay an old signed envelope. See the [catalog-only recovery steps](../../docs/development/releases.md#recover-a-catalog-only-failure).
