# macOS

The manager runs on macOS 11 or newer, on Apple silicon and Intel. It is experimental.

## Installing apps

Each Craft app publishes one universal, notarized `*-macos-universal.dmg`. The manager uses it for both release formats:

- **Installer** (default) copies the app into `/Applications`, or into `~/Applications` if `/Applications` is not writable. Apps you installed yourself are detected by their bundle name and `ai.storyteller.*` bundle identifier. Uninstall removes the app bundle.
- **Portable app** copies the app into the library under `releases/<app>/`.

Before installing, the manager checks GitHub's SHA-256 digest for the DMG. It then verifies the copied app with `codesign --verify --deep --strict` and Gatekeeper (`spctl --assess`).

The library is stored in `~/Library/Application Support/Craft Apps Manager`. Hourly checks use launch agents in `~/Library/LaunchAgents/io.github.craft-apps-manager.*.plist`, and notifications appear through Notification Center.

## Manager updates

Craft Apps Manager.app can update itself. The new ZIP is checked against GitHub's SHA-256 digest, extracted into a private `.craft-manager-update-<unique id>` folder beside the manager bundle and verified with `codesign`. Staging and rollback stay on the manager's filesystem even when the library is on another volume.

After the manager closes, the old app bundle is moved into that folder as `previous.app` before the replacement is published. Failed swaps restore the previous manager when possible; if restoration fails, the previous bundle stays at that recovery location. The helper checks whether macOS accepts the restart request; this does not confirm that the application finished starting.

A failed update is reported on the next manager startup, with a recovery path, and recorded outside the signed bundle in `~/Library/Application Support/Craft Apps Manager/self-update-result.json` and in the staging folder's `result.json`. Older library-based plans are rejected; reopen the manager and download again. The app must be in a folder you can write to, such as Applications. Development builds run outside a bundle and cannot update themselves.

Updates come from the GitHub repository set by `CRAFT_MANAGER_REPOSITORY` (owner/name) at build time. The default is `CryptoKey98/craft-apps-manager`. CI sets it to the repository being built, so a fork's packages update from the fork's releases.

## Not yet available on macOS

- Deleting app profile data on uninstall. The upstream profile paths have not been verified yet.
- Shortcut files. App bundles open directly from Finder.

## Building

Install Rust with rustup and the Xcode Command Line Tools. Then build:

```text
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo test --locked -- --test-threads=1
./scripts/package-macos.sh
```

The script writes a universal `Craft Apps Manager.app` and a ZIP to `dist/macos`. The bundle has only an ad-hoc signature and is not notarized. The first time a downloaded copy is opened, macOS blocks it. Click **Done**, then go to **System Settings → Privacy & Security** and click **Open Anyway**. Right-click → **Open** no longer skips this check on macOS 15 and newer.

Source builds use Homebrew for extra tools: `sevenzip`, plus `node`, `cmake` and `nasm` for ArtCraft X. They produce a bare executable under `builds/`, not an app bundle.
