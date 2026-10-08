#!/bin/sh
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$project/target/release/craft-apps-manager"}
output=${2:-"$project/dist/linux"}
binary=$(realpath "$binary")
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$project/Cargo.toml" | head -1)
case "$version" in ''|*[!0-9.]*) echo "Invalid package version" >&2; exit 1;; esac
command -v dpkg-deb >/dev/null
command -v dpkg-shlibdeps >/dev/null
machine=$(od -An -tu2 -j18 -N2 "$binary" | tr -d ' ')
header=$(od -An -tx1 -N6 "$binary" | tr -d ' \n')
case "$header:$machine" in
    7f454c460101:3) architecture=i386;;
    7f454c460201:62) architecture=amd64;;
    7f454c460201:183) architecture=arm64;;
    *) echo "Unsupported ELF architecture" >&2; exit 1;;
esac
mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd)
package="$output/craft-apps-manager_${version}_${architecture}.deb"
[ ! -e "$package" ] || { echo "Package already exists: $package" >&2; exit 1; }
work=$(mktemp -d "$output/deb-build.XXXXXX")
mkdir -p "$work/debian" "$work/package/DEBIAN" "$work/package/usr/bin" \
    "$work/package/usr/share/applications" "$work/package/usr/share/pixmaps" \
    "$work/package/usr/share/doc/craft-apps-manager/licenses/app-icons"
cat > "$work/debian/control" <<EOF
Source: craft-apps-manager
Section: utils
Priority: optional
Maintainer: CryptoKey98 <cryptokey98@users.noreply.github.com>

Package: craft-apps-manager
Architecture: any
Description: Manage Craft app releases, sources and builds
EOF
# Derive linked-library version requirements from the actual executable.
# GUI backends loaded dynamically need explicit dependencies as well.
linked=$(cd "$work" && dpkg-shlibdeps -O -e"$binary")
linked=$(printf '%s\n' "$linked" | sed -n 's/^shlibs:Depends=//p')
[ -n "$linked" ] || { echo "Could not determine ELF dependencies" >&2; exit 1; }
cat > "$work/package/DEBIAN/control" <<EOF
Package: craft-apps-manager
Version: $version
Architecture: $architecture
Section: utils
Priority: optional
Maintainer: CryptoKey98 <cryptokey98@users.noreply.github.com>
Homepage: https://github.com/CryptoKey98/craft-apps-manager
Depends: $linked, libx11-6, libxcursor1, libxi6, libxrandr2, libxkbcommon0, libwayland-client0, libwayland-cursor0, libwayland-egl1, libegl1, libgl1, libnotify-bin:native, xdg-utils:native, pkexec:native
Recommends: 7zip:native
Description: Manage Craft app releases, sources and builds
 Download Craft releases, check for updates, manage source archives,
 and build executables. Settings and libraries remain in each user's
 data directory and are kept when this package is removed.
EOF
install -m 0755 "$binary" "$work/package/usr/bin/craft-apps-manager"
install -m 0644 "$project"/packaging/linux/*.desktop "$work/package/usr/share/applications/"
install -m 0644 "$project/assets/icon.png" "$work/package/usr/share/pixmaps/craft-apps-manager.png"
install -m 0644 "$project/LICENSE" "$project/THIRD-PARTY-NOTICES.txt" "$project/docs/linux.md" "$work/package/usr/share/doc/craft-apps-manager/"
install -m 0644 "$project"/assets/app-icons/*.txt "$work/package/usr/share/doc/craft-apps-manager/licenses/app-icons/"
dpkg-deb --root-owner-group -Zxz -z9 --build "$work/package" "$package"
dpkg-deb --info "$package" >/dev/null
case "$work" in "$output"/deb-build.*) rm -rf -- "$work";; *) echo "Unexpected staging path; leaving it intact" >&2; exit 1;; esac
printf '%s\n' "$package"
