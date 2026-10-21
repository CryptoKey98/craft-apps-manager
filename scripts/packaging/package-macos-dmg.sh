#!/bin/sh
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
selected=$(python3 "$project/scripts/lib/updater_config.py" format --target-os macos --package-format dmg)
[ "$selected" = true ] || { echo "DMG packaging is disabled in updater.toml"; exit 0; }
bundle=${1:?Pass the bundle already verified by package-macos.sh}
output=${2:?Pass the output DMG path}
[ ! -e "$output" ] || { echo "DMG already exists" >&2; exit 1; }
python3 "$project/scripts/lib/updater_config.py" verify --target-os macos --binary "$bundle/Contents/MacOS/craft-apps-manager" --require-universal
codesign --verify --deep --strict "$bundle"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
# Write Finder metadata directly so packaging works without a logged-in Finder
# or Automation permission. Keep these build dependencies out of the app.
python3 -m venv "$stage/venv"
"$stage/venv/bin/python" -m pip --disable-pip-version-check install \
    -r "$project/packaging/macos/dmg-requirements.txt"
"$stage/venv/bin/dmgbuild" -s "$project/packaging/macos/dmg-settings.py" \
    -D "app=$bundle" -D "background=$project/packaging/macos/dmg-background.tiff" \
    'Craft Apps Manager' "$output"
hdiutil verify "$output"
printf '%s\n' "$output"
