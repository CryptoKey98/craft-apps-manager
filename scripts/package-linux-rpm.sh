#!/bin/sh
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$project/target/release/craft-apps-manager"}
output=${2:-"$project/dist/linux"}
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$project/Cargo.toml" | head -1)
case "$version" in ''|*[!0-9.]*) echo "Invalid package version" >&2; exit 1;; esac
command -v rpmbuild >/dev/null
[ -f "$binary" ] || { echo "Build the native release executable first" >&2; exit 1; }
# Read the ELF machine field rather than the host CPU: an x86_64 host can
# package a cross-compiled i686 executable.
machine=$(od -An -tu2 -j18 -N2 "$binary" | tr -d ' ')
header=$(od -An -tx1 -N6 "$binary" | tr -d ' \n')
case "$header:$machine" in 7f454c460101:3) architecture=i686;; 7f454c460201:62) architecture=x86_64;; 7f454c460201:183) architecture=aarch64;; *) echo "Unsupported ELF architecture" >&2; exit 1;; esac
mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd)
work=$(mktemp -d "$output/rpm-build.XXXXXX")
# Keep the staging directory on failure so the packaging log can be inspected.
mkdir -p "$work/SOURCES" "$work/SPECS" "$work/staged/licenses/app-icons"
cp "$binary" "$work/staged/craft-apps-manager"
cp "$project/assets/icon.png" "$work/staged/icon.png"
cp "$project/LICENSE" "$project/THIRD-PARTY-NOTICES.txt" "$work/staged/"
cp "$project/docs/linux.md" "$work/staged/linux.md"
cp "$project"/assets/app-icons/*.txt "$work/staged/licenses/app-icons/"
cp "$project"/packaging/linux/*.desktop "$work/staged/"
cp "$project/packaging/linux/craft-apps-manager.spec" "$work/SPECS/"
tar -czf "$work/SOURCES/package.tar.gz" -C "$work/staged" .
# The executable is prebuilt and this spec has no BuildRequires. Avoid querying
# or creating an RPM database during this packaging-only operation.
rpmbuild -bb --nodeps --target "$architecture" --define "_topdir $work" --define "manager_version $version" --define "_build_id_links none" "$work/SPECS/craft-apps-manager.spec"
find "$work/RPMS" -type f -name '*.rpm' -exec cp {} "$output/" \;
printf '\nRPM packages: %s\n' "$output"
case "$work" in "$output"/rpm-build.*) rm -rf -- "$work";; *) echo "Unexpected staging path; leaving it intact" >&2; exit 1;; esac
