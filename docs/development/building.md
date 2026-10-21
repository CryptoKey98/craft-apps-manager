# Building and testing

For contributors compiling the manager. To build a Craft app through the UI, see [Using the manager](../user-guide/using-the-manager.md#build-apps-and-manage-backups). Packaged manager releases do not require Rust.

Run the commands below from the repository root. Use stable Rust installed through [rustup](https://rust-lang.org/tools/install/).

## Prerequisites

| Platform | Required for compiling the manager |
| --- | --- |
| Windows | Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK |
| Ubuntu/Debian | C/C++ build tools, pkg-config, OpenSSL and the native GUI development libraries below |
| Fedora | C/C++ build tools, pkgconf, OpenSSL and the matching X11/Wayland/OpenGL development libraries |
| Arch | `base-devel`, `pkgconf`, `openssl`, `libx11`, `libxi`, `libxrandr`, `libxcursor`, `libxkbcommon`, `wayland`, `libglvnd` |
| macOS | Xcode Command Line Tools; both Rust targets below for a universal bundle |

Ubuntu/Debian development dependencies:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxinerama-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev libssl-dev
```

These are build-machine dependencies. User installation requirements are in the [Linux](../user-guide/linux.md) and [macOS](../user-guide/macos.md) guides. Upstream Craft apps may require additional tools; the manager's tool setup handles supported app/platform combinations.

## Choose the updater

The updater choice is compiled into the executable. There is no end-user setting to switch it.

### Default GitHub-only build

With the checked-in `updater.toml` and no updater environment overrides:

```sh
cargo build --release --locked --bin craft-apps-manager
```

This keeps the original GitHub release discovery and update path. Enabling `--features service-updater` alone does not activate the service.

### Official service-updater build

On Windows:

```powershell
python scripts/lib/updater_config.py build --config updater.production.toml --target-os windows
python scripts/lib/updater_config.py verify --config updater.production.toml --target-os windows --binary target/release/craft-apps-manager.exe
```

On Linux:

```sh
python3 scripts/lib/updater_config.py build --config updater.production.toml --target-os linux
python3 scripts/lib/updater_config.py verify --config updater.production.toml --target-os linux --binary target/release/craft-apps-manager
```

On macOS, use the Linux commands with `--target-os macos`. A bare executable is useful for development; use the bundle packaging instructions below to test macOS self-updates.

The helper runs Cargo with the selected TOML and required feature. Verification checks the compiled version, revision, updater source, repository, channel, public-key fingerprint and runtime configuration fingerprint. No private signing key is needed to build a client.

Both build choices normally write the **same output file**:

| Platform | Executable |
| --- | --- |
| Windows | `target/release/craft-apps-manager.exe` |
| Linux/macOS | `target/release/craft-apps-manager` |

The last build replaces the earlier build choice. Explicit `--target` builds go under `target/<triple>/release`; `CARGO_TARGET_DIR` can change the base directory. Pass the same configuration each time you want a service build. Forks should supply their own TOML, repository and trust configuration; see [Updater configuration](updater-operations.md#source-build-choice).

Launch the executable with `--builder` for the separate Builder window. For an isolated library, add `--root /absolute/path/to/test-library` (use a Windows absolute path on Windows).

## Tests and static checks

Close Manager and Builder before tests that acquire the shared operation lock. Run tests serially:

```sh
cargo fmt --check
cargo test --locked --all-targets -- --test-threads=1
cargo clippy --locked --all-targets -- -D warnings
```

For changes affecting the service updater, also run:

```sh
cargo test --locked --all-targets --features service-updater -- --test-threads=1
cargo clippy --locked --all-targets --features service-updater -- -D warnings
```

Some tests require separately selected fixtures or tools and are intentionally ignored. Compilation and unit tests do not prove GUI behavior or actual installer transactions on another platform. See [Updater checks](updater-operations.md#local-checks) for protocol and Worker tests. Do not run native installer smoke tests against a real installation without a disposable test setup.

## Package a local build

Keep the same updater configuration for compilation and packaging; packaging rejects a mismatched executable fingerprint. For example, set `CRAFT_UPDATER_CONFIG=updater.production.toml` in the packaging process's environment when packaging the official service build. On PowerShell use `$env:CRAFT_UPDATER_CONFIG = 'updater.production.toml'`.

Scripts refuse to overwrite existing portable archives. Use a new output directory or intentionally remove only disposable package output, never a library or an installed app.

### Windows

Build for `x86_64-pc-windows-msvc` or `i686-pc-windows-msvc` before running packaging scripts. For example:

```powershell
python scripts/lib/updater_config.py build --config updater.production.toml --target-os windows --target x86_64-pc-windows-msvc
```

Portable and MSI packaging require a matching 7-Zip SDK folder with its license files. MSI packaging also needs WiX. See [Windows packaging](windows-packaging.md) for commands and prerequisites.

### Linux

Build a native executable first. The portable scripts create a ZIP and then reuse its exact payload for TAR.GZ:

```sh
sh scripts/packaging/package-linux-zip.sh target/release/craft-apps-manager
sh scripts/packaging/package-linux-tar.sh dist/linux/Craft-Apps-Manager-1.0.0-linux-x64.zip
```

Replace the archive's version and architecture with the actual output. TAR.GZ packaging currently requires ZIP in the selected package configuration.

| Format | Script | Additional tools |
| --- | --- | --- |
| DEB | `scripts/packaging/package-linux-deb.sh` | `dpkg-dev` on Ubuntu/Debian |
| RPM | `scripts/packaging/package-linux-rpm.sh` | `rpmbuild`, supplied by `rpm-build` on Fedora |
| Arch | `scripts/packaging/package-linux-arch.sh` | Arch build environment; run as a regular user |

Each native script accepts the executable path as its first argument and writes to `dist/linux` by default. Architectures are detected from the binary. DEB/RPM dependencies are derived from linked libraries; a local build on a newer distribution may require newer libraries than the published build.

Published ZIP/TAR.GZ/DEB/RPM builds use Ubuntu 22.04; Arch packages are built separately on Arch. Linux x86 launch testing has used multilib on 64-bit Ubuntu/Fedora VMs, not a native 32-bit operating system. Linux ARM64 has not been qualified.

To cross-build x86 on Ubuntu x64, enable the `i386` package architecture and install `gcc-multilib`, `libc6-dev-i386`, and matching GUI development libraries with `:i386`. Add `i686-unknown-linux-gnu` with rustup. Use `PKG_CONFIG_ALLOW_CROSS=1` and `PKG_CONFIG_LIBDIR=/usr/lib/i386-linux-gnu/pkgconfig:/usr/share/pkgconfig` for that target build. This does not create x86 releases for upstream apps that only publish x64.

### macOS

Build and package on macOS:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
./scripts/packaging/package-macos.sh
sh scripts/packaging/package-macos-dmg.sh 'dist/macos/Craft Apps Manager.app' dist/macos/Craft-Apps-Manager-1.0.0-macos-universal.dmg
```

The first script compiles both architectures, combines them into a universal bundle, verifies both embedded updater identities, applies an ad-hoc signature, and creates a ZIP with `portable.txt`. The DMG script uses that same bundle in a drag-to-Applications layout. Adjust the example version to match Cargo.toml.

DMG packaging needs Python 3.9+ with `venv` and access to PyPI. Pinned `dmgbuild` dependencies are installed into a temporary environment and removed afterward. These are packaging requirements, not prerequisites for running the app. Bundles are not Apple-notarized; see [First launch](../user-guide/macos.md#first-launch).

## Wiki documentation preview

Wiki pages are generated from the existing Markdown; edit the source guides, not the generated pages. Python 3.10+ and Git are sufficient. No extra Python packages, Rust build, or wiki credentials are needed for local checks.

From the repository root on PowerShell:

```powershell
$sourceCommit = git rev-parse HEAD
python scripts/testing/test_wiki_documentation.py
python scripts/documentation/wiki.py check --source-commit $sourceCommit
```

On Linux/macOS:

```sh
python3 scripts/testing/test_wiki_documentation.py
python3 scripts/documentation/wiki.py check --source-commit "$(git rev-parse HEAD)"
```

The preview is written to `target/wiki-preview/`. Open `Home.md` or `_Sidebar.md` to review the content and navigation. These are generated Markdown files, not a local copy of GitHub's wiki renderer. Image and page URLs target their eventual wiki locations; those links will not serve the unpublished preview. Source links point to the selected commit, so uncommitted edits are only visible in the local preview until committed.

`scripts/documentation/wiki-manifest.json` lists the public pages and images to include. Add a new guide there deliberately. Unlisted files, credentials, and test output are excluded. Existing source files remain unchanged by generation.

Local preview commands and Git tests cannot publish. The documentation workflow validates pull requests without publishing credentials. Enabled main-branch runs then sync the wiki; no app release is required. Syncing preserves unrelated pages and refuses to overwrite directly edited generated files. Move intended corrections into the source docs before resolving a conflict.

### Wiki publication setup

Repository maintainers configure this once:

1. Create an initial `Home` page through GitHub's Wiki tab if the wiki has never been initialized.
2. Create the `wiki-publication` Actions environment and restrict deployments to the `main` branch. Add a dedicated wiki-write credential as the environment secret `WIKI_SYNC_TOKEN`. A classic personal access token with `public_repo` scope works for this public repository. Track its expiration and replace the secret before it expires.
3. Set the repository variable `WIKI_SYNC_ENABLED` to `true`. To pause publishing, set it to `false`.
4. For the first sync only, inspect the existing Home page and set the repository variable `WIKI_BOOTSTRAP_HOME_SHA256` to the SHA-256 of its exact Git blob bytes. This approves replacement of that page alone. Remove the variable after successful publication. Other pre-existing page collisions require manual reconciliation.
5. Run the **Documentation** workflow manually with branch `main`. Later documentation changes merged into main trigger it automatically.

Only the publication job receives the credential, through its environment. It serializes wiki writes, checks that the source still matches main, and uses a normal Git push. Concurrent wiki edits or stale source commits stop publication rather than overwrite newer work. A failed sync does not affect app releases; resolve the reported conflict or credential problem and rerun the workflow from current main. Editing a generated wiki page directly can block later syncing, so keep corrections in the source guides.

## Next steps

- [Release preparation](releases.md): validate version metadata and publish through GitHub Actions.
- [Updater operations](updater-operations.md): configure a fork, signing, hosting, and update routes.
- [Architecture](../README.md#understand-the-architecture): installation and operation design references.
