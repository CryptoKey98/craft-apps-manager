"""Validate release inputs without compiling or publishing anything."""
import argparse
from pathlib import Path
import re


def release_notes(root, tag):
    match = re.fullmatch(r"v(\d+)\.(\d+)\.(\d+)", tag)
    if not match or any(int(part) > limit for part, limit in zip(match.groups(), (255, 255, 65535))):
        raise ValueError("Use a stable vMAJOR.MINOR.PATCH tag within Windows MSI limits")
    version = tag[1:]
    manifest = (root / "Cargo.toml").read_text(encoding="utf-8")
    package = re.search(r"(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)", manifest)
    if not package or not re.search(r'^version\s*=\s*"' + re.escape(version) + r'"\s*$', package[1], re.M):
        raise ValueError("Release tag does not match Cargo.toml")
    lock = (root / "Cargo.lock").read_text(encoding="utf-8")
    managers = [entry for entry in re.split(r"(?m)^\[\[package\]\]\s*$", lock)
                if re.search(r'^name\s*=\s*"craft-apps-manager"\s*$', entry, re.M)]
    if len(managers) != 1 or not re.search(r'^version\s*=\s*"' + re.escape(version) + r'"\s*$', managers[0], re.M):
        raise ValueError("Cargo.lock manager version does not match the release tag")
    readme = (root / "README.md").read_text(encoding="utf-8")
    if readme.count(f"Version **{version}** includes") != 1:
        raise ValueError("README current-version caption does not match the release tag")
    changelog = (root / "CHANGELOG.md").read_text(encoding="utf-8")
    headings = list(re.finditer(r"(?m)^## ([^\n]+)$", changelog))
    if not headings or headings[0][1] != version:
        raise ValueError("Latest changelog heading does not match the release tag")
    end = headings[1].start() if len(headings) > 1 else len(changelog)
    notes = changelog[headings[0].end():end].strip()
    if not notes or "Describe the changes before merging this release preparation." in notes:
        raise ValueError("Release notes are empty or still contain the preparation placeholder")
    return notes + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--notes-file", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    try:
        notes = release_notes(root, args.tag)
    except (ValueError, OSError) as error:
        parser.exit(1, f"Release validation failed: {error}\n")
    args.notes_file.write_text(notes, encoding="utf-8")
    print(f"Validated {args.tag}: versions agree and release notes are ready")


if __name__ == "__main__":
    main()
