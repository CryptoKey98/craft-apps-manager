# Production catalog inputs

`standby.json` records the independently downloaded and SHA-256-verified packages from the already published v0.9.0 release. It intentionally has **no update routes**. It is bootstrap policy, not the 1.0.0 release catalog. `supported-clients.json` covers the first service-enabled clients on Windows x64/x86 (ZIP/MSI), Linux x64/x86 (ZIP/DEB/RPM, plus Arch x64), and macOS x64/arm64 bundles. Those 13 profiles all return Current against standby policy; no downgrade is offered.

The signed envelope is generated into ignored output and deployed separately. Never place private keys here. Do not replace the referenced v0.9.0 package bytes. Native package layout descriptors are inert to native handlers; ZIP layouts were inspected in the actual archives.

`release-1.0.0.json` now contains the candidate policy generated from the verified 1.0.0 draft packages. `updater.production.toml` points to it for release publication; it is not the currently served policy. The draft is built from merged main commit `f72cddb3d1078db339b1889b647351f945b4578e`. Do not deploy this policy until its packages are public. Normal renewals continue copying the verified live standby policy, not this candidate. Before releasing, re-run preflight and remote verification. The release preflight rejects this standby policy for tag v1.0.0. Do not invent future hashes or use test-repository packages. Keep the v0.9.0 release containing the stable `updates-v2.json` metadata mirror available across future releases.

## Renewal

The production configuration uses 30-day signed validity with renewal starting seven days before expiry. The daily workflow runs at 08:17 UTC (01:17 Arizona time). It authenticates the previous catalog, validates supported clients with the Rust router, downloads and hashes each package, then signs a higher sequence using the pinned key. The same envelope bytes are deployed to the Worker and GitHub mirror and read back for verification.

Repository variables: `CRAFT_UPDATE_CATALOG_URL`, `CRAFT_UPDATE_CATALOG_MIRROR`, `CLOUDFLARE_ACCOUNT_ID`, `CRAFT_UPDATE_CATALOG_RENEW_ENABLED=true`, and `CRAFT_UPDATE_V2_DEPLOY_ENABLED=true`. Keep `CRAFT_UPDATE_V2_ENABLED=false` until the official service release is intentionally activated. Scheduled renewal and client activation are separate gates.

`CRAFT_UPDATE_SIGNING_KEY` is a repository secret. `CLOUDFLARE_API_TOKEN` is an environment secret in `manager-update-production`; it needs Workers Scripts Edit and Account Settings Read for the selected account. No zone access is needed for workers.dev. Do not log either secret. Rotate the Cloudflare token before any chosen expiration.

The renewal workflow and all its configuration/library dependencies must reach the default branch before GitHub can run the schedule. To verify the deployment credential before merging, manually run the existing Check workflow on the implementation branch with `production_catalog_only=true`. It calls the same renewal workflow with `force_renew=true`, preserving the current signed policy and skipping application builds. This signs a higher sequence, redeploys the same standby offers, and verifies the Worker and mirror; it never creates an application release. A local successful renewal alone does not prove GitHub can use its stored Cloudflare credential.


## Preparing a draft

Use the manual Release workflow with revision `1`, platform `all`, `publish_baseline=false` and `prepare_draft=true`. It requires an existing draft and validates configuration, signing key, scope and current-client routing before building. This draft-only path omits the final release-tag policy check because package hashes do not exist yet; it cannot publish the application or deploy production catalog offers. The application source is the immutable `source_ref`, independently of the workflow definition's branch. Use a source commit containing any publisher-tool fixes needed for that run.

Draft assets are authenticated through GitHub's release ID because a draft may have no public Git tag. Preserve immutable package upload checks; do not clobber signed assets. Verify uploaded package bytes and signatures before publishing. A locally signed candidate is verification evidence only: publication must fetch the current live catalog again, increment its latest sequence, re-verify packages and sign fresh, rather than reusing a potentially stale candidate envelope. The stable GitHub mirror remains on v0.9.0, not duplicated on every release.

## Shared release packages

Revision 1 uses the existing compatibility filenames for both signed protocols and manual downloads: no separate `update` copies. All service package uploads use hash/size collision checks and never clobber an existing package. Later service-enabled revisions use distinct `-Rev-N` package names in the signed v1 and v2 metadata; baseline URLs remain unchanged. Older manifest-aware managers follow the signed revision filename. Managers predating revision manifests can install the compatible baseline first, then update with the new updater. GitHub-only publishers retain the existing compatibility-refresh behavior. Keep any historical package required by a supported bridge route.
