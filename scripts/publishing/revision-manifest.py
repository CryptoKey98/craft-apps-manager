"""Prepare signed revision metadata and compatibility downloads; never upload."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time


def target(name):
    match = re.search(r"-(windows|linux)-(x64|x86)\.zip$", name)
    if match:
        return match[1], match[2], "zip"
    if name.endswith("-macos-universal.zip"):
        return "macos", "universal", "zip"
    match = re.search(r"-windows-(x64|x86)\.msi$", name)
    if match:
        return "windows", match[1], "msi"
    match = re.search(r"_(amd64|i386)\.deb$", name)
    if match:
        return "linux", {"amd64": "x64", "i386": "x86"}[match[1]], "deb"
    match = re.search(r"\.(x86_64|i686)\.rpm$", name)
    if match:
        return "linux", {"x86_64": "x64", "i686": "x86"}[match[1]], "rpm"
    if name.endswith("-x86_64.pkg.tar.zst"):
        return "linux", "x64", "arch"
    raise ValueError(f"Unrecognized manager package: {name}")


def compatibility_name(version, platform, architecture, kind):
    if kind in ("zip", "msi"):
        return f"Craft-Apps-Manager-{version}-{platform}-{architecture}.{kind}"
    if kind == "deb":
        arch = {"x64": "amd64", "x86": "i386"}[architecture]
        return f"craft-apps-manager_{version}_{arch}.deb"
    if kind == "rpm":
        arch = {"x64": "x86_64", "x86": "i686"}[architecture]
        return f"craft-apps-manager-{version}-0.2.{arch}.rpm"
    return f"craft-apps-manager-{version}-1-x86_64.pkg.tar.zst"


def native_version(file, kind):
    if kind == "deb":
        return subprocess.check_output(["dpkg-deb", "-f", str(file), "Version"], text=True).strip()
    if kind == "rpm":
        return subprocess.check_output(["rpm", "-qp", "--queryformat", "%{VERSION}-%{RELEASE}", str(file)], text=True).strip()
    if kind == "arch":
        metadata = subprocess.check_output(["bsdtar", "-xOf", str(file), ".PKGINFO"], text=True)
        return next(line.split(" = ", 1)[1] for line in metadata.splitlines() if line.startswith("pkgver = "))
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--packages", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--revision", type=int, required=True)
    parser.add_argument("--immutable-packages", action="store_true", help="Keep baseline URLs unchanged; sign distinct revision filenames")
    parser.add_argument("--previous", type=Path)
    parser.add_argument("--platform", choices=["windows", "linux", "macos", "all"], default="all")
    parser.add_argument("--updater-config", type=Path)
    args = parser.parse_args()
    from lib.updater_config import load_view
    effective = load_view("publisher", "linux" if args.platform == "all" else args.platform, args.updater_config, {"repository": args.repository, "revision": args.revision})
    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version) or not 1 <= args.revision <= 65535:
        parser.error("Invalid version or revision")
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
    key = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(os.environ["CRAFT_UPDATE_SIGNING_KEY"]))
    expected_key = bytes.fromhex(effective["public_key_hex"])
    if key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw) != expected_key:
        raise ValueError("Signing key does not match the key embedded in the manager")
    packages = {}
    if args.previous:
        envelope = json.loads(args.previous.read_text())
        Ed25519PublicKey.from_public_bytes(expected_key).verify(bytes.fromhex(envelope["signature"]), envelope["payload"].encode())
        previous = json.loads(envelope["payload"])
        if previous["schema"] != 1 or previous["repository"] != args.repository or previous["version"] != args.version:
            raise ValueError("Previous manifest belongs to another release")
        packages = {(p["platform"], p["architecture"], p["format"]): p for p in previous["packages"]}
    elif args.revision != 1:
        raise ValueError("A hotfix needs the previous signed manifest")
    # Extra formats are not part of v1 metadata, but their signed v2 URLs
    # must be equally immutable across revisions.
    if args.immutable_packages and args.revision > 1:
        for extra in sorted(args.packages.iterdir()):
            suffix = next((s for s in (".tar.gz", ".dmg") if extra.name.endswith(s)), None)
            if suffix:
                expected_platform = "macos" if suffix == ".dmg" else "linux"
                if args.platform not in ("all", expected_platform):
                    raise ValueError("Unexpected platform in hotfix upload")
                destination = extra.with_name(extra.name[:-len(suffix)] + f"-Rev-{args.revision}" + suffix)
                with extra.open("rb") as source, destination.open("xb") as target_file:
                    shutil.copyfileobj(source, target_file)
                extra.unlink()
    inputs = sorted(p for p in args.packages.iterdir() if p.suffix in (".zip", ".msi", ".deb", ".rpm", ".zst"))
    seen = set()
    for file in inputs:
        identity = target(file.name)
        if args.platform != "all" and identity[0] != args.platform:
            raise ValueError("Unexpected platform in hotfix upload")
        if identity in seen:
            raise ValueError("Duplicate build target")
        seen.add(identity)
        old = packages.get(identity)
        if old and args.revision <= old["revision"]:
            raise ValueError("Revision must advance for every affected target")
        legacy = args.packages / compatibility_name(args.version, *identity)
        if args.immutable_packages and args.revision > 1:
            # Old manifest-aware clients use this signed filename directly.
            # Pre-manifest clients retain the baseline, which can then self-update.
            suffix = next(s for s in (".pkg.tar.zst", ".zip", ".msi", ".deb", ".rpm") if legacy.name.endswith(s))
            legacy = legacy.with_name(legacy.name[:-len(suffix)] + f"-Rev-{args.revision}" + suffix)
        if identity[2] == "deb" and args.revision > 1 and not args.immutable_packages:
            # Older managers require exact Version metadata; sign this same
            # compatible package for newer managers as well.
            work = args.packages / ("compat-deb-" + identity[1])
            subprocess.run(["dpkg-deb", "-R", str(file), str(work)], check=True)
            control = work / "DEBIAN/control"
            control.write_text(re.sub(r"(?m)^Version:.*$", "Version: " + args.version, control.read_text()))
            subprocess.run(["dpkg-deb", "--build", "--root-owner-group", str(work), str(legacy)], check=True)
            shutil.rmtree(work)
        elif file != legacy:
            shutil.copyfile(file, legacy)
        if file != legacy:
            file.unlink()
        packages[identity] = dict(platform=identity[0], architecture=identity[1], format=identity[2], revision=args.revision, asset=legacy.name, size=legacy.stat().st_size, sha256=hashlib.sha256(legacy.read_bytes()).hexdigest(), package_version=native_version(legacy, identity[2]))
    if not seen:
        raise ValueError("No packages to publish")
    payload = json.dumps(dict(schema=1, repository=args.repository, version=args.version, expires=int(time.time()) + 365 * 86400, packages=list(packages.values())), separators=(",", ":"))
    envelope = dict(payload=payload, signature=key.sign(payload.encode()).hex())
    (args.packages / "updates-v1.json").write_text(json.dumps(envelope, separators=(",", ":")))


if __name__ == "__main__":
    main()
