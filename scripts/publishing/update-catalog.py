#!/usr/bin/env python3
"""Validate and sign v2 catalogs locally. Never upload or deploy."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.parse
import urllib.request

ROOT = project_root()
MAX_SEQUENCE = 9007199254740991

def validate_shape(catalog):
    if catalog.get("schema") != 2 or type(catalog.get("sequence")) is not int or not 1 <= catalog["sequence"] <= MAX_SEQUENCE:
        raise ValueError("Invalid catalog schema/sequence")
    for name, limit in (("releases", 128), ("packages", 512), ("routes", 512)):
        values = catalog[name]
        if len(values) > limit or len({v["id"] for v in values}) != len(values):
            raise ValueError("Catalog bound/duplicate ID: " + name)
    releases = {r["id"] for r in catalog["releases"]}
    packages = {p["id"] for p in catalog["packages"]}
    if any(p["release"] not in releases for p in catalog["packages"]) or any(r["target"] not in packages for r in catalog["routes"]):
        raise ValueError("Missing catalog reference")

def retain_bridges(catalog, previous):
    current = {p["id"]: p for p in catalog["packages"]}
    routes = {r["id"]: r for r in catalog["routes"]}
    by_url = {p['url']: p for p in catalog['packages']}
    for p in previous["packages"]:
        if p['url'] in by_url and (p['sha256'], p['size']) != (by_url[p['url']]['sha256'], by_url[p['url']]['size']):
            raise ValueError('A published v2 asset URL must remain immutable')
        if p["id"] in current and current[p["id"]] != p:
            raise ValueError("Immutable package identity changed")
    for route in previous["routes"]:
        if route["bridge"] and (route["target"] not in current or routes.get(route["id"]) != route):
            raise ValueError("Required historical bridge removed or changed")

def renew(catalog, sequence, issued, validity_days=30):
    if not 2 <= validity_days <= 30: raise ValueError("Invalid catalog validity")
    if type(sequence) is not int or not catalog["sequence"] < sequence <= MAX_SEQUENCE:
        raise ValueError("Sequence must strictly increase")
    result = copy.deepcopy(catalog)
    result.update(sequence=sequence, issued=issued, expires=issued + validity_days * 86400)
    validate_shape(result)
    return result

def payload_bytes(catalog):
    return json.dumps(catalog, separators=(",", ":"), ensure_ascii=False).encode("utf-8")

def verified_previous(path, public):
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    envelope = json.loads(path.read_text(encoding="utf-8"))
    Ed25519PublicKey.from_public_bytes(public).verify(bytes.fromhex(envelope["signature"]), envelope["payload"].encode())
    catalog = json.loads(envelope["payload"])
    validate_shape(catalog)
    return catalog

def validate_routes(config, profiles, validator):
    command = [str(validator)] if validator else ["cargo", "run", "--locked", "--quiet", "--features", "release-tools,service-updater", "--bin", "update-catalog-tool", "--"]
    subprocess.run(command + [str(config.resolve()), str(profiles.resolve())], cwd=ROOT, check=True)

def publication_preflight(effective, validator, signing_key=None):
    """Local readiness gate before any release upload; remote bytes are checked later."""
    if effective["source"] != "service" or not effective["publishing"]["enabled"]:
        raise ValueError("Activated release requires enabled service publication")
    catalog_path=Path(effective["next_catalog_path"])
    profiles_path=Path(effective["profiles_path"])
    catalog=json.loads(catalog_path.read_text(encoding="utf-8"))
    profiles=json.loads(profiles_path.read_text(encoding="utf-8"))
    validate_shape(catalog)
    release_tag = os.environ.get("CRAFT_RELEASE_TAG")
    if release_tag and not any(r.get("tag") == release_tag for r in catalog["releases"]):
        raise ValueError("Publication catalog does not contain the requested release tag")
    expected=(effective["repository"],effective["channel"])
    if (catalog.get("repository"),catalog.get("channel")) != expected:
        raise ValueError("Publication catalog scope differs from compiled updater")
    if not isinstance(profiles,list) or not profiles or any(
            not isinstance(p,dict) or (p.get("repository"),p.get("channel")) != expected for p in profiles):
        raise ValueError("Publication profile scope differs from compiled updater")
    if signing_key is not None:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
        public=Ed25519PrivateKey.from_private_bytes(bytes.fromhex(signing_key)).public_key().public_bytes(Encoding.Raw,PublicFormat.Raw)
        if public.hex() != effective["public_key_hex"]:
            raise ValueError("Signing key does not match pinned application key")
    validate_routes(catalog_path,profiles_path,validator)


class AssetRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        url = urllib.parse.urlsplit(newurl)
        if url.scheme != "https" or url.hostname not in {"github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"} or url.username or url.password:
            raise ValueError("Untrusted package redirect")
        return super().redirect_request(request, fp, code, msg, headers, newurl)

def verify_assets(catalog):
    opener = urllib.request.build_opener(AssetRedirect())
    for package in catalog["packages"]:
        if package["id"] in catalog.get("withdrawn", []): continue
        url = urllib.parse.urlsplit(package["url"])
        if url.scheme != "https" or url.hostname != "github.com" or not url.path.startswith("/" + catalog["repository"] + "/releases/download/") or url.username or url.password:
            raise ValueError("Untrusted asset origin")
        digest = hashlib.sha256()
        size = 0
        with opener.open(urllib.request.Request(package["url"], headers={"User-Agent": "CraftUpdatePublisher/2"}), timeout=30) as response:
            while True:
                chunk = response.read(65536)
                if not chunk: break
                size += len(chunk)
                if size > package["size"]: raise ValueError("Remote asset exceeds signed size")
                digest.update(chunk)
        if size != package["size"] or digest.hexdigest() != package["sha256"].lower():
            raise ValueError("Remote asset size/hash mismatch")
    return {"catalog_sha256": hashlib.sha256(payload_bytes(catalog)).hexdigest(), "verified_at": int(time.time())}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["validate", "verify-assets", "build", "preflight"])
    parser.add_argument("--config", type=Path)
    parser.add_argument("--profiles", type=Path)
    parser.add_argument("--validator", type=Path)
    parser.add_argument("--previous", type=Path)
    parser.add_argument("--initial", action="store_true")
    parser.add_argument("--sequence", type=int)
    parser.add_argument("--issued", type=int, default=int(time.time()))
    parser.add_argument("--signing-key-env", default="CRAFT_UPDATE_SIGNING_KEY")
    parser.add_argument("--verified-assets", type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--updater-config", type=Path)
    args = parser.parse_args()
    if args.command == "preflight":
        from lib.updater_config import load_view
        effective=load_view("publisher","linux",args.updater_config)
        publication_preflight(effective,args.validator,os.environ[args.signing_key_env])
        return
    if not args.config or not args.profiles:
        parser.error("--config and --profiles required")
    catalog = json.loads(args.config.read_text(encoding="utf-8"))
    validate_shape(catalog)
    validate_routes(args.config, args.profiles, args.validator)
    if args.command == "validate": return
    if not args.out: parser.error("--out required")
    if args.command == "verify-assets":
        args.out.write_text(json.dumps(verify_assets(catalog)), encoding="utf-8")
        return
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
    from lib.updater_config import load_view
    effective = load_view("publisher", "linux", args.updater_config)
    if not effective["publishing"]["enabled"]: raise ValueError("Catalog publication is disabled in updater.toml")
    if (catalog["repository"],catalog["channel"]) != (effective["repository"],effective["channel"]): raise ValueError("Catalog scope differs from compiled updater configuration")
    public = bytes.fromhex(effective["public_key_hex"])
    key = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(os.environ[args.signing_key_env]))
    if key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw) != public:
        raise ValueError("Signing key does not match pinned application key")
    if not args.previous and not args.initial: parser.error("--previous required; --initial is only for first activation")
    if args.previous and args.initial: parser.error("Choose previous or initial, not both")
    previous = verified_previous(args.previous, public) if args.previous else None
    if previous:
        if (previous["repository"], previous["channel"]) != (catalog["repository"], catalog["channel"]): raise ValueError("Previous scope mismatch")
        retain_bridges(catalog, previous)
    if args.sequence is None or not (previous["sequence"] if previous else 0) < args.sequence <= MAX_SEQUENCE:
        raise ValueError("Invalid new sequence")
    if not args.verified_assets: parser.error("--verified-assets required before signing")
    receipt = json.loads(args.verified_assets.read_text())
    if receipt["catalog_sha256"] != hashlib.sha256(payload_bytes(catalog)).hexdigest() or not 0 <= int(time.time()) - receipt["verified_at"] <= 3600:
        raise ValueError("Asset verification receipt is stale or belongs to different bytes")
    catalog.update(sequence=args.sequence, issued=args.issued, expires=args.issued + effective["publishing"]["catalog_validity_days"] * 86400)
    payload = payload_bytes(catalog)
    if len(payload) > 1024 * 1024: raise ValueError("Catalog too large")
    envelope = {"payload": payload.decode(), "signature": key.sign(payload).hex()}
    if args.out.exists(): raise ValueError("Refusing to overwrite catalog output")
    args.out.write_text(json.dumps(envelope, separators=(",", ":")), encoding="utf-8")

if __name__ == "__main__":
    try: main()
    except Exception:
        # Do not dump credential-bearing environment values or private key material.
        raise SystemExit("Catalog validation/signing failed; no catalog published") from None
