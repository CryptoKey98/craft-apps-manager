#!/bin/sh
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$project/target/release/craft-apps-manager"}
output=${2:-"$project/dist/linux"}
[ -f "$binary" ] || { echo "Build the Linux executable first" >&2; exit 1; }
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$project/Cargo.toml" | head -1)
case "$version" in ''|*[!0-9.]*) echo "Invalid package version" >&2; exit 1;; esac
machine=$(od -An -tu2 -j18 -N2 "$binary" | tr -d ' ')
header=$(od -An -tx1 -N6 "$binary" | tr -d ' \n')
case "$header:$machine" in
    7f454c460101:3) architecture=x86;;
    7f454c460201:62) architecture=x64;;
    7f454c460201:183) architecture=arm64;;
    *) echo "Unsupported ELF architecture" >&2; exit 1;;
esac
mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd)
archive="$output/Craft-Apps-Manager-$version-linux-$architecture.zip"
[ ! -e "$archive" ] || { echo "Package already exists: $archive" >&2; exit 1; }
python3 - "$project" "$binary" "$archive" <<'PY'
import pathlib, sys, zipfile
project, binary, archive = map(pathlib.Path, sys.argv[1:])
files = [(binary, 'craft-apps-manager'), (project/'assets/icon.png', 'icon.png')]
files += [(project/name, name) for name in ('README.md', 'CHANGELOG.md', 'LICENSE', 'THIRD-PARTY-NOTICES.txt')]
files += [(project/'docs/linux.md', 'docs/linux.md')]
files += [(p, 'licenses/app-icons/'+p.name) for p in sorted((project/'assets/app-icons').glob('*.txt'))]
files += [(project/'assets/fonts/OFL.txt', 'licenses/fonts/IBM-Plex-OFL.txt')]
try:
    with zipfile.ZipFile(archive, 'x', zipfile.ZIP_DEFLATED, compresslevel=9) as package:
        for source, name in files:
            package.write(source, 'Craft Apps Manager Linux/'+name)
    with zipfile.ZipFile(archive) as package:
        if package.testzip() is not None:
            raise RuntimeError('ZIP verification failed')
except Exception:
    archive.unlink(missing_ok=True)
    raise
PY
printf '%s\n' "$archive"
