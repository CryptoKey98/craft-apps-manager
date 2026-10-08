#!/bin/sh
# Builds a universal (Apple silicon + Intel) Craft Apps Manager.app and a ZIP of it.
# Set CRAFT_MANAGER_REPOSITORY=owner/name to self-update from a fork's releases.
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:-"$project/dist/macos"}
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$project/Cargo.toml" | head -1)
case "$version" in ''|*[!0-9.]*) echo "Invalid package version" >&2; exit 1;; esac
cd "$project"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked --target "$target"
done
mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd)
bundle="$output/Craft Apps Manager.app"
archive="$output/Craft-Apps-Manager-$version-macos-universal.zip"
[ ! -e "$archive" ] || { echo "Package already exists: $archive" >&2; exit 1; }
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
lipo -create -output "$bundle/Contents/MacOS/craft-apps-manager" \
    target/aarch64-apple-darwin/release/craft-apps-manager \
    target/x86_64-apple-darwin/release/craft-apps-manager
iconset=$(mktemp -d)/icon.iconset
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" assets/icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" assets/icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$bundle/Contents/Resources/icon.icns"
rm -rf "$(dirname "$iconset")"
cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Craft Apps Manager</string>
  <key>CFBundleDisplayName</key><string>Craft Apps Manager</string>
  <key>CFBundleIdentifier</key><string>io.github.craft-apps-manager</string>
  <key>CFBundleExecutable</key><string>craft-apps-manager</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
for name in README.md CHANGELOG.md LICENSE THIRD-PARTY-NOTICES.txt; do
    cp "$name" "$bundle/Contents/Resources/"
done
mkdir -p "$bundle/Contents/Resources/licenses/app-icons"
cp assets/app-icons/*.txt "$bundle/Contents/Resources/licenses/app-icons/"
# Ad-hoc signature: required on Apple silicon. Not notarized, so the first
# launch of a downloaded copy needs right-click > Open.
codesign --force --sign - "$bundle"
codesign --verify --strict "$bundle"
ditto -c -k --norsrc --noextattr --keepParent "$bundle" "$archive"
printf '%s\n' "$bundle" "$archive"
