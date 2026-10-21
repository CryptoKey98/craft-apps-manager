#!/bin/sh
# Reuse the exact tested ZIP payload; keep the legacy ZIP as a separate download.
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
selected=$(python3 "$project/scripts/lib/updater_config.py" format --target-os linux --package-format tar.gz)
[ "$selected" = true ] || { echo "tar.gz packaging is disabled in updater.toml"; exit 0; }
archive=${1:?Pass the Linux ZIP built by package-linux-zip.sh}
python3 - "$archive" <<'PY'
import io, pathlib, sys, tarfile, zipfile
source = pathlib.Path(sys.argv[1])
target = source.with_suffix('.tar.gz')
if target.exists(): raise SystemExit('Refusing to overwrite portable tar.gz')
try:
    with zipfile.ZipFile(source) as zip, tarfile.open(target, 'x:gz', format=tarfile.USTAR_FORMAT) as tar:
        for entry in zip.infolist():
            info = tarfile.TarInfo(entry.filename.rstrip('/'))
            info.mode = ((entry.external_attr >> 16) & 0o777) or 0o644
            if entry.is_dir():
                info.type = tarfile.DIRTYPE
                tar.addfile(info)
            else:
                info.size = entry.file_size
                with zip.open(entry) as data: tar.addfile(info, data)
    with tarfile.open(target, 'r:gz') as tar:
        if not any(m.name.endswith('/craft-apps-manager') and m.mode & 0o111 for m in tar):
            raise ValueError('Missing executable manager in tar.gz')
except Exception:
    target.unlink(missing_ok=True)
    raise
print(target)
PY
