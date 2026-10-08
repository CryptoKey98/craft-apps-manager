# macOS

The manager runs on macOS 11 or newer, on Apple silicon and Intel. It is experimental.

## Installing apps

Each Craft app publishes one universal, notarized `*-macos-universal.dmg`. The manager uses it for both release formats:

- **Installer** (default) copies the app into `/Applications`, or into `~/Applications` if `/Applications` is not writable. Apps you installed yourself are detected by their bundle name and `ai.storyteller.*` bundle identifier. Uninstall removes the app bundle.
- **Portable app** copies the app into the library under `releases/<app>/`.

Before installing, the manager checks GitHub's SHA-256 digest for the DMG. It then verifies the copied app with `codesign --verify --deep --strict` and Gatekeeper (`spctl --assess`).

The library is stored in `~/Library/Application Support/Craft Apps Manager`. Hourly checks use launch agents in `~/Library/LaunchAgents/io.github.craft-apps-manager.*.plist`, and notifications appear through Notification Center.

## Manager updates

Craft Apps Manager.app can update itself. The new ZIP is checked against GitHub's SHA-256 digest, extracted into the library's `runtime/self-update` folder and verified with `codesign`. After the manager closes, the old app bundle is replaced. The previous bundle is kept in that folder as `previous.app`. The app must be in a folder you can write to, such as Applications. Development builds run outside a bundle and cannot update themselves.

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

The script writes a universal `Craft Apps Manager.app` and a ZIP to `dist/macos`. The bundle has only an ad-hoc signature and is not notarized. A downloaded copy needs right-click → **Open** the first time.

Source builds use Homebrew for extra tools: `sevenzip`, plus `node`, `cmake` and `nasm` for ArtCraft X. They produce a bare executable under `builds/`, not an app bundle.
