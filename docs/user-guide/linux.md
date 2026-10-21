# Linux

For installing and using the manager on Linux. Compilation and packaging are covered in [Building and testing](../development/building.md).

## Choose a download

Get a package from [Releases](https://github.com/CryptoKey98/craft-apps-manager/releases/latest). Choose x64 for a 64-bit Intel/AMD system; x86/i386/i686 packages are 32-bit. Linux ARM64 has not been verified.

| System | Native package | Portable alternative |
| --- | --- | --- |
| Ubuntu/Debian | `.deb` | ZIP or TAR.GZ |
| Fedora/RPM-based distributions | `.rpm` | ZIP or TAR.GZ |
| Arch/Arch-compatible distributions | `.pkg.tar.zst` (experimental, x64 only) | ZIP or TAR.GZ |

The manager's portable archive is different from the **AppImage** format available for many Craft apps.

## Install a native package

Open a terminal in the download folder. These examples use the 1.0.0 x64 filenames; substitute your downloaded file.

Ubuntu/Debian:

```sh
sudo apt install ./craft-apps-manager_1.0.0_amd64.deb
```

Fedora:

```sh
sudo dnf install ./craft-apps-manager-1.0.0-0.2.x86_64.rpm
```

Arch:

```sh
sudo pacman -U ./craft-apps-manager-1.0.0-1-x86_64.pkg.tar.zst
```

Packages include Manager and Builder application-menu entries. Native package installation and removal require administrator approval. Removing the manager package keeps your library and settings.

On Arch, keep the system fully updated. Tool setup uses `pacman -S --needed` without a separate database refresh; do not perform a partial system upgrade.

## Use a portable archive

Extract the **whole** ZIP or TAR.GZ into a writable folder, then open `craft-apps-manager` inside `Craft Apps Manager Linux`. For example:

```sh
tar -xzf Craft-Apps-Manager-1.0.0-linux-x64.tar.gz
cd 'Craft Apps Manager Linux'
./craft-apps-manager
```

Keep `portable.txt` and the other extracted files together. If your ZIP extractor lost executable permissions, mark the manager executable in your file manager or run `chmod +x craft-apps-manager` from its folder. Do not run the manager itself with sudo.

New portable libraries use `data` beside the executable. Native installations use `$XDG_DATA_HOME/craft-apps-manager`, or `~/.local/share/craft-apps-manager` when unset. The Data path shows the active location. See [Storage and portability](using-the-manager.md#storage-and-portability) before moving files.

## Install and update apps

Choose each app's available format and architecture in its installation dialog. AppImages run in extraction mode, so FUSE is not required. Installer mode uses the distribution's native package family; DEB and RPM are not installed on Arch. Upstream apps must publish the selected format.

Custom native packages need inspection tools:

| Package | Tools used |
| --- | --- |
| DEB | `dpkg-deb`, `bsdtar` (Ubuntu/Debian package `libarchive-tools`) |
| RPM | `rpm`, `rpm2cpio`, `bsdtar` |
| Arch | `bsdtar` (Arch package `libarchive`) |

The manager records package identity and a package-owned desktop executable. If several launchers are found, choose the app to track. Unsupported shell wrappers may require manual installation.

Manager updates use a matching native package for native installations, or a supported portable archive for portable installations. Native transactions use PolicyKit and apt, dnf, or pacman. Cancellation may have to wait for a package transaction to finish. Settings and library locations are retained.

## Background checks and system tray

Startup, periodic, and scheduled checks notify you; they do not automatically install updates. Hourly scheduled checks use systemd user timers during your session.

Close-to-tray needs a usable tray host and an X11/XWayland window. Some GNOME desktops require a compatible tray extension. The manager prefers a reachable X11 display; on native Wayland the setting is disabled and closing quits normally. Without a usable tray, closing also quits so the manager cannot become inaccessible.

## Limitations and help

- Arch packages and Linux desktop integration remain experimental. Tested desktops include Ubuntu, Fedora, and Arch; behavior can differ elsewhere.
- Published portable/DEB/RPM builds use Ubuntu 22.04. Local builds on newer distributions may need newer libraries.
- x86 launch tests used multilib on 64-bit systems; native 32-bit systems and Linux ARM64 have not been qualified.
- Profile deletion is disabled where app profile locations have not been verified.

For app actions, backups, builds, and troubleshooting, see [Using the manager](using-the-manager.md) and the [documentation index](../README.md).
