# Linux development build

The Windows and Linux applications share the same Rust source. Linux testing and packages use their own folders and app library.

## Current support

The first test system is Ubuntu 26.04 x86_64. AppImage releases are the default; Installer mode selects Debian packages on Ubuntu/Debian and RPM packages on Fedora/RHEL-family distributions. AppImage launch uses extraction mode, so FUSE is not required. Package installation and removal ask for desktop administrator authorization.

App updates, source downloads, backups, logs, launch settings and the source builder use the Linux platform layer. Hourly checks use systemd user timers and send desktop notifications; they never install updates automatically. Timers run during the user's session.

Profile deletion remains disabled on Linux until each app's actual profile directories have been checked. Installer cancellation is available during downloading, but an authorized package transaction must finish to protect the package database.

## Folders

By default, the manager keeps its library in `$XDG_DATA_HOME/craft-apps-manager`, or `~/.local/share/craft-apps-manager` when that variable is unset. Use `--root /absolute/path` for a separate test library. Sources, builds, releases, backups and logs remain separate inside the library.

## Build

On Ubuntu, install the native GUI dependencies:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxinerama-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev libssl-dev p7zip-full
```

Install Rust using the instructions at https://rust-lang.org/tools/install/, then run:

```sh
cargo build --release --locked
cargo test --locked --all-targets -- --test-threads=1
cargo clippy --locked --all-targets -- -D warnings
```

The executable is `target/release/craft-apps-manager`. Launch it with `--builder` to open the builder. Set up build tools installs the selected app's prerequisites on Ubuntu/Debian and Fedora/RHEL-family systems with DNF; ArtCraft X additionally needs Node, npm and its desktop dependencies. Other distributions currently require manual prerequisite setup.

## Distribution testing

Both x64 and x86 (i686) manager builds have passed their source tests on Ubuntu 26.04. The x86 Manager and Builder also launch on Ubuntu 26.04 and Fedora 44 with the 32-bit runtime libraries installed. These are multilib tests on 64-bit VMs, not tests of a native 32-bit operating system. Fedora dependency resolution for the i686 RPM has been checked.

To cross-build x86 on an Ubuntu x64 host, add the `i386` package architecture and install `gcc-multilib`, `libc6-dev-i386`, and the GUI development packages above with `:i386`. Add the Rust target with `rustup target add i686-unknown-linux-gnu`, then build with `PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_LIBDIR=/usr/lib/i386-linux-gnu/pkgconfig:/usr/share/pkgconfig cargo build --release --locked --target i686-unknown-linux-gnu`. The packaging scripts detect the binary architecture rather than the build host.

Run `sh scripts/package-linux-zip.sh path/to/craft-apps-manager` for a portable ZIP. The 32-bit manager only selects matching upstream x86 app assets; it cannot turn an upstream x64-only release into a 32-bit app.

Linux packages are experimental. The current binaries were built on Ubuntu 26.04 and require glibc 2.43 or newer. Ubuntu 26.04 and Fedora 44 x86_64 have been used for testing. This does not establish compatibility with older systems. The proposed Linux CI job builds on Ubuntu 22.04 to keep the glibc baseline older; that job and additional distribution testing are needed before claiming compatibility with older distributions. An Ubuntu 22.04 or 24.04 VM is a useful next test target. macOS is not implemented in this port.

## Ubuntu and Debian development package

Run `sh scripts/package-linux-deb.sh path/to/craft-apps-manager` on Ubuntu or Debian. It requires `dpkg-dev` and creates an `amd64` or `i386` DEB based on the executable's ELF architecture. Linked-library requirements are calculated from the binary, so a package built on a newer Ubuntu release may require newer system libraries.

Install the local package with `sudo apt install ./craft-apps-manager_0.4.0_amd64.deb` (or the `i386` package). The package includes both desktop launchers, the icon, and license notices. Removing it keeps the user's library and settings.

## Fedora RPM development package

Build a native Linux release binary, then run:

```sh
sh scripts/package-linux-rpm.sh target/release/craft-apps-manager
```

The script requires `rpmbuild` (Fedora package `rpm-build`). It places the development RPM in `dist/linux`, with a system executable, manager and builder menu entries, icon, documentation and license notices. The RPM automatically records ELF library requirements; packages built on a newer distribution may not install on older systems.

On Fedora, install the local RPM using:

```sh
sudo dnf install ./craft-apps-manager-0.4.0-0.2*.x86_64.rpm
```

RPM packages are experimental during distribution testing. Removing the package leaves each user's app library intact. For a DEB/RPM installation, the manager update button downloads the matching native package and invokes apt-get or dnf with an administrator prompt. It verifies the installed package version before restarting, and preserves user settings and libraries. Portable ZIP installations use the separate executable-replacement update flow and require a writable installation folder.
