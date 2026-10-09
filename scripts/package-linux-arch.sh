#!/bin/sh
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$project/target/release/craft-apps-manager"}
output=${2:-"$project/dist/linux"}
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$project/Cargo.toml" | head -1)
case "$version" in ''|*[!0-9.]*) echo "Invalid package version" >&2; exit 1;; esac
[ "$(id -u)" != 0 ] || { echo "Run makepkg as a regular user, not root" >&2; exit 1; }
command -v makepkg >/dev/null
[ -f "$binary" ] || { echo "Build the Linux executable first" >&2; exit 1; }
header=$(od -An -tx1 -N6 "$binary" | tr -d ' \n')
machine=$(od -An -tu2 -j18 -N2 "$binary" | tr -d ' ')
[ "$header:$machine" = '7f454c460201:62' ] || { echo "Arch package requires an x86_64 Linux executable" >&2; exit 1; }
mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd)
work=$(mktemp -d "$output/arch-build.XXXXXX")
mkdir -p "$work/payload/licenses/app-icons" "$work/payload/licenses/fonts"
cp "$binary" "$work/payload/craft-apps-manager"
cp "$project/assets/icon.png" "$work/payload/icon.png"
cp "$project/LICENSE" "$project/THIRD-PARTY-NOTICES.txt" "$work/payload/"
cp "$project/docs/linux.md" "$work/payload/linux.md"
cp "$project"/assets/app-icons/*.txt "$work/payload/licenses/app-icons/"
cp "$project/assets/fonts/OFL.txt" "$work/payload/licenses/fonts/IBM-Plex-OFL.txt"
cp "$project"/packaging/linux/*.desktop "$work/payload/"
tar -czf "$work/payload.tar.gz" -C "$work/payload" .
hash=$(sha256sum "$work/payload.tar.gz" | cut -d ' ' -f1)
printf 'pkgver=%s\n' "$version" > "$work/PKGBUILD"
cat "$project/packaging/linux/PKGBUILD" >> "$work/PKGBUILD"
printf '\nsha256sums=(%s)\n' "$hash" >> "$work/PKGBUILD"
(cd "$work" && PKGEXT='.pkg.tar.zst' makepkg --force)
cp "$work/craft-apps-manager-$version-1-x86_64.pkg.tar.zst" "$output/"
case "$work" in "$output"/arch-build.*) rm -rf -- "$work";; *) echo "Unexpected staging path" >&2; exit 1;; esac
printf '\nArch package: %s/craft-apps-manager-%s-1-x86_64.pkg.tar.zst\n' "$output" "$version"
