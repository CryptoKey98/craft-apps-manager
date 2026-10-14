# Manager update revisions

The manager compares its software version and an embedded build revision. Revision 1 is the normal release and has no visible revision suffix. Hotfixes advance only the affected platform and package targets; About and update dialogs show `Rev. 2` or higher.

Download names follow `Craft Apps Manager-0.8.3-windows-x64.zip` for the baseline and `Craft Apps Manager-0.8.3-Rev-2-windows-x64.zip` for a hotfix. The same naming applies to macOS and Linux, with the appropriate architecture and package extension. Legacy hyphenated compatibility filenames remain available for older managers. The friendly baseline download remains immutable when a compatibility download is refreshed.

## Publishing

1. Publish a normal release with revision support, beginning with 0.8.3 revision 1.
2. Commit the platform fix without changing Cargo's software version.
3. Run the Release workflow manually. Choose the existing release tag, affected platform, a higher revision, and the complete hotfix commit SHA.
4. The workflow builds the selected platform, uploads uniquely named revision packages, refreshes compatibility downloads, and uploads the signed `updates-v1.json` manifest last.

Windows and macOS revisions are supported alongside Linux. Linux packages advance their native package revision; Windows MSI uses its upgrade transaction to replace the previous same-version product. Native installer testing is still necessary before distributing a new installer revision.

Only affected targets advance. Unaffected platforms retain their manifest entries and receive no update notification. Never delete previous revision packages during publication. Failed publication leaves the previous manifest active.

Publication is serialized for each release tag. Checksum lists contain immutable revision packages; hotfixes add a separate platform/revision checksum list instead of replacing the baseline list. Compatibility downloads change, so verify those using the current GitHub asset digest rather than an older checksum list.

## Older managers

Compatibility downloads keep the filenames expected by older managers and contain the newest executable. Older Debian updaters additionally require exact software-version metadata, so the workflow creates a compatibility DEB containing the same new revision executable with that metadata. The revision-specific DEB advances the native package revision.

An older manager can therefore update directly to the newest revision in one installation. New managers select immutable revision-specific downloads. After restarting, the embedded revision prevents the same update from being offered again. Cached GitHub release metadata can briefly delay detection after publishing.

## Signing

The public verification key is in `assets/update-public-key.hex` and is embedded in the executable. Configure the matching private 32-byte Ed25519 seed, encoded as hexadecimal, in the repository Actions secret `CRAFT_UPDATE_SIGNING_KEY`. Never commit or publish the private key.

For the initial local implementation, the private seed is held in the ignored `.release-secrets/update-signing-key.hex` file. Preserve a secure backup of this private key. The GitHub secret must be configured before publishing 0.8.3. Forks must create their own signing key pair.

The updater checks the signature, repository, release version, expiration, target identity, asset size, and SHA-256. Missing targets mean no update. Invalid metadata fails safely. Automatic updates never move to a lower version or revision. The manifest expires after a year; renew it with signed metadata before expiration if a release remains current that long.

## Linux hotfix test

The Ubuntu close-to-tray and AppImage-label changes are intentionally deferred to Linux revision 2. Keep them out of the 0.8.3 revision-1 baseline. Validate both revision 1 → revision 2 and an older manager → the revision-2 compatibility download, using isolated test data.
