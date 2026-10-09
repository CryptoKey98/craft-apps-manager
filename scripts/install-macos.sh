#!/bin/bash
# Installs or updates Craft Apps Manager on macOS from the latest GitHub release.
#   curl -fsSL https://raw.githubusercontent.com/CryptoKey98/craft-apps-manager/main/scripts/install-macos.sh | bash
set -euo pipefail
repo="CryptoKey98/craft-apps-manager"
name="Craft Apps Manager.app"

release="$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest")"
url_of() {
    printf '%s\n' "$release" |
        grep -o '"browser_download_url": *"[^"]*"' |
        sed 's/.*"\(https:[^"]*\)"/\1/' |
        grep -E "$1" | head -1
}
package_url="$(url_of '/Craft-Apps-Manager-[0-9.]+-macos-universal\.zip$')"
sums_url="$(url_of '/SHA256SUMS\.txt$')"
if [ -z "$package_url" ] || [ -z "$sums_url" ]; then
    echo "The latest release has no macOS package." >&2
    exit 1
fi
package="$(basename "$package_url")"
case "$package_url" in
    "https://github.com/$repo/releases/download/"*) ;;
    *) echo "Unexpected download location: $package_url" >&2; exit 1 ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
echo "Downloading $package"
curl -fL --progress-bar -o "$work/$package" "$package_url"
curl -fsSL -o "$work/SHA256SUMS.txt" "$sums_url"
(cd "$work" && grep " $package\$" SHA256SUMS.txt | shasum -a 256 -c -)

ditto -x -k "$work/$package" "$work/unpacked"
codesign --verify --strict "$work/unpacked/$name"

destination="/Applications"
if [ ! -w "$destination" ]; then
    destination="$HOME/Applications"
    mkdir -p "$destination"
fi
if pgrep -f "$name/Contents/MacOS/craft-apps-manager" >/dev/null; then
    echo "Quit Craft Apps Manager before updating it." >&2
    exit 1
fi
rm -rf "$destination/$name"
ditto "$work/unpacked/$name" "$destination/$name"
# Downloads made by curl are not quarantined; clear the flag in case a copy was.
xattr -dr com.apple.quarantine "$destination/$name" 2>/dev/null || true
echo "Installed $destination/$name"
open "$destination/$name"
