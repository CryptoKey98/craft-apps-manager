#!/usr/bin/env python3
"""Keep immutable v2 downloads separate from mutable legacy compatibility assets."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import argparse
import json
from pathlib import Path
import shutil

SUFFIXES = ('.pkg.tar.zst', '.tar.gz', '.zip', '.msi', '.deb', '.rpm', '.dmg')

def stage(source, output, revision, pattern):
    if not 1 <= revision <= 65535: raise ValueError('Invalid build revision')
    output.mkdir(parents=True, exist_ok=True)
    names = {}
    for file in sorted(source.iterdir()):
        if not file.is_file(): continue
        suffix = next((s for s in SUFFIXES if file.name.endswith(s)), None)
        if not suffix: continue
        stem = file.name[:-len(suffix)]
        name = pattern.format(stem=stem, suffix=suffix, revision=revision, revision_suffix='' if revision == 1 else '-Rev-' + str(revision))
        if name in ('', '.', '..') or '/' in name or '\\' in name or ':' in name or not name.endswith(suffix): raise ValueError('Unsafe immutable asset name')
        if name == file.name: raise ValueError('v2 immutable name collides with a mutable legacy asset')
        with file.open('rb') as original, (output / name).open('xb') as target:
            shutil.copyfileobj(original, target)
        names[file.name] = name
    if not names: raise ValueError('No immutable assets staged')
    return names

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--revision', type=int, required=True)
    parser.add_argument('--config', type=Path)
    args = parser.parse_args()
    from lib.updater_config import publisher_config
    config = publisher_config(args.config)
    print(json.dumps(stage(args.source, args.output, args.revision, config['asset_name_pattern']), indent=2))

if __name__ == '__main__': main()
