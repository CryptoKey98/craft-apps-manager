#!/bin/sh
# End-to-end check on macOS with real upstream releases: app discovery,
# disk image installs in both release formats (including ArtCraft) and removal.
# Run after scripts/package-macos.sh.
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
manager="${1:-$project/dist/macos/Craft Apps Manager.app}/Contents/MacOS/craft-apps-manager"
library="${RUNNER_TEMP:-/tmp}/craft-smoke-$$"
rm -rf "$library"
mkdir -p "$library"
run() {
    echo "+ craft-apps-manager $*" >&2
    if ! "$manager" --root "$library" "$@"; then
        tail -n 40 "$library/logs/updates.log" 2>/dev/null || true
        exit 1
    fi
}

run --list-apps > "$library/apps.json"
for key in cadcraft deckcraft gridcraft soundcraft wordcraft printcraft artcraft; do
    grep -q "\"key\": \"$key\"" "$library/apps.json" || { echo "App list misses $key" >&2; exit 1; }
done

# Library (portable) format: the bundle is copied out of the disk image.
run --release-format portable --install-app soundcraft
ls -d "$library/releases/soundcraft/"*.app
run --release-format portable --install-app printcraft
ls -d "$library/releases/printcraft/"*.app
for key in soundcraft printcraft; do
    run --uninstall-app "$key"
done

# Applications folder format, for a crafting app and for ArtCraft.
for key in soundcraft artcraft; do
    run --release-format installer --install-app "$key"
done
ls -d /Applications/*.app ~/Applications/*.app 2>/dev/null | grep -i -E 'soundcraft|artcraft'
for key in soundcraft artcraft; do
    run --uninstall-app "$key"
done
if ls -d /Applications/*.app ~/Applications/*.app 2>/dev/null | grep -i -q -E 'soundcraft|artcraft'; then
    echo "Apps were not removed" >&2
    exit 1
fi
echo "macOS end-to-end check passed"
