#!/usr/bin/env python3
"""Build/run isolated Windows updates through the test Worker; never touches releases."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import zipfile
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

ROOT = project_root()
SERVICE = ROOT / "update-service"
BASE = SERVICE / ".wrangler/transactions"
PUBLIC = SERVICE / ".wrangler/test-public"
MARKER = "Craft updater isolated fixture"
WORKER = "https://craft-apps-manager-updates-test.cryptokeyofficial.workers.dev/"


def snapshot(root):
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in root.rglob("*") if p.is_file()}


def assert_preserved(root, before):
    for name, digest in before.items():
        path = root / name
        assert path.is_file(), "Missing preserved file: " + str(path)
        assert hashlib.sha256(path.read_bytes()).hexdigest() == digest, "Changed preserved file: " + str(path)


def windows_records(artifacts):
    packages = []
    for suffix, version in [("b", "1.1.0"), ("c", "1.2.0")]:
        size, digest = artifacts[version]
        asset = f"worker-fixture-{suffix}.zip"
        packages.append(dict(id=f"{suffix}-windows", release=suffix, revision=1,
            platform="windows", architecture="x64", installation="portable",
            handler="archive.zip.v1", required_capabilities=["archive.zip.v1", "layout.manager.v1"],
            asset=asset, size=size, sha256=digest,
            url=f"https://github.com/owner/manager/releases/download/v{version}/{asset}",
            layout=dict(schema=1, root=f"Windows Fixture {suffix.upper()}", entrypoint="fixture-manager.exe")))
    routes = []
    for suffix, source in [("b", "1.0.0"), ("c", "1.1.0")]:
        routes.append(dict(id=f"windows-to-{suffix}", min_source=dict(version=source, revision=1),
            max_source=dict(version=source, revision=65535), platform="windows", architecture="x64",
            installation="portable", required_capabilities=["archive.zip.v1"], target=f"{suffix}-windows",
            bridge=suffix == "b", destination_capabilities=["archive.zip.v1", "layout.manager.v1"]))
    return dict(packages=packages, routes=routes)


def prepare():
    if os.name != "nt":
        raise RuntimeError("This initial live replacement scenario requires Windows")
    BASE.mkdir(parents=True, exist_ok=True)
    (BASE / ".craft-updater-fixture").write_text(MARKER)
    binaries = BASE / "binaries"
    binaries.mkdir(exist_ok=True)
    for version in ("1.0.0", "1.1.0", "1.2.0"):
        subprocess.run(["cargo", "build", "--locked", "--features", "updater-test-fixture", "--bin", "update-worker-fixture"],
            cwd=ROOT, env=dict(os.environ, CRAFT_UPDATER_FIXTURE_VERSION=version), check=True)
        shutil.copy2(ROOT / "target/debug/update-worker-fixture.exe", binaries / f"{version}.exe")
    subprocess.run(["node", "--experimental-strip-types", "scripts/prepare-test-deployment.ts"], cwd=SERVICE, check=True)
    envelope = json.loads((PUBLIC / "v2/catalog.json").read_text())
    catalog = json.loads(envelope["payload"])
    artifacts = {}
    for suffix, version in [("b", "1.1.0"), ("c", "1.2.0")]:
        archive = BASE / f"worker-fixture-{suffix}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as package:
            package.write(binaries / f"{version}.exe", f"Windows Fixture {suffix.upper()}/fixture-manager.exe")
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        artifacts[version] = (archive.stat().st_size, digest)
        (PUBLIC / "artifacts").mkdir(exist_ok=True)
        shutil.copy2(archive, PUBLIC / "artifacts" / f"{digest}-{archive.name}")
    additions = windows_records(artifacts)
    catalog["packages"].extend(additions["packages"])
    catalog["routes"].extend(additions["routes"])
    # Higher than the just-generated catalog. No same-sequence content changes.
    catalog["sequence"] += 1
    payload = json.dumps(catalog, separators=(",", ":"))
    key = Ed25519PrivateKey.from_private_bytes(bytes([7]) * 32)
    (PUBLIC / "v2/catalog.json").write_text(json.dumps(dict(payload=payload, signature=key.sign(payload.encode()).hex())))
    (PUBLIC / "index.html").write_text('<!doctype html><meta charset="utf-8"><title>Updater test service</title><h1>Updater test service</h1><p>Isolated signed routing and Windows fixture packages. No public application releases. Production managers reject this test key.</p>')
    (BASE / "prepared.json").write_text(json.dumps(dict(sequence=catalog["sequence"], artifacts=artifacts), indent=2))
    print("Prepared isolated Worker fixture binaries/packages. Test Worker deployment is a separate explicit command.")


def wait_module():
    spec = importlib.util.spec_from_file_location("fixture_wait", ROOT / "scripts/testing/test-updater-transactions.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def receipt(data, version, wait):
    deadline = time.monotonic() + 75
    while time.monotonic() < deadline:
        try:
            value = json.loads((data / "worker-fixture-receipt.json").read_text())
            if value["version"] == version:
                wait.wait_exit(value["pid"])
                return value
        except (FileNotFoundError, json.JSONDecodeError):
            pass
        time.sleep(0.1)
    raise RuntimeError("No completed restart receipt for " + version)


def run_case(name, source, versions, wait):
    case = BASE / (name + "-" + str(time.time_ns()))
    install, data, external = (case / value for value in ("install", "data", "external-app"))
    for directory in (case, install, data, external):
        directory.mkdir(parents=True, exist_ok=False)
        (directory / ".craft-updater-fixture").write_text(MARKER)
    target = install / "fixture-manager.exe"
    shutil.copy2(BASE / f"binaries/{source}.exe", target)
    app = data / "apps/Fixture App/app.exe"
    app.parent.mkdir(parents=True)
    shutil.copy2(BASE / "binaries/1.1.0.exe", app)
    external_app = external / "app.exe"
    shutil.copy2(BASE / "binaries/1.1.0.exe", external_app)
    (app.parent / "project.txt").write_text("Preserve this app project and user settings")
    (external / "project.txt").write_text("External installation must remain untouched")
    records = [dict(name="fixture-app", version="0.1.0", path=str(path.parent), architecture="x64", install_kind="portable", installation_label=label)
        for path, label in [(app, "A"), (external_app, "B")]]
    (data / "settings.json").write_text(json.dumps(dict(appsRoot=str(data / "apps"), apps=records[:1], installations=records)))
    (data / "manager-settings.json").write_text(json.dumps(dict(theme="light", checkInstalledAppsPeriodically=True, appCheckIntervalMinutes=20)))
    (data / "portable.txt").write_text("Preserve portable marker")
    subprocess.run([str(target), "--root", str(data)], check=True, timeout=30)
    before, outside = snapshot(data), snapshot(external)
    # Restart receipts change intentionally, everything seeded as user data must not.
    before.pop("worker-fixture-receipt.json")
    previous = hashlib.sha256(target.read_bytes()).hexdigest()
    denied = subprocess.run([str(target), "--root", str(data), "--worker", WORKER], capture_output=True, timeout=30)
    assert denied.returncode != 0 and b"explicit approval" in denied.stderr, "Unapproved hop was accepted"
    assert hashlib.sha256(target.read_bytes()).hexdigest() == previous
    refusals = []
    for mutation in ("source-hash", "package-bytes"):
        output = subprocess.check_output([str(target), "--root", str(data), "--worker", WORKER, "--approve", "--stage-only"], timeout=75)
        plan = Path(output.decode().strip())
        if mutation == "source-hash":
            auth = plan.parent / "fixture-authorization.json"
            content = json.loads(auth.read_text())
            content["source_hash"] = "0" * 64
            auth.write_text(json.dumps(content))
        else:
            artifact = plan.parent / "download.package"
            content = bytearray(artifact.read_bytes())
            content[-1] ^= 1
            artifact.write_bytes(content)
        rejection = subprocess.run([str(plan.parent / "update-helper.exe"), "--apply-self-update", str(plan)], capture_output=True, timeout=75)
        assert rejection.returncode != 0, "Tampered fixture update was accepted"
        assert hashlib.sha256(target.read_bytes()).hexdigest() == previous
        assert_preserved(data, before)
        assert_preserved(external, outside)
        refusals.append(dict(case=mutation, result="rejected", error=rejection.stderr.decode(errors="replace").strip()))
    hops = []
    for version in versions:
        subprocess.run([str(target), "--root", str(data), "--worker", WORKER, "--approve"], check=True, timeout=75)
        restarted = receipt(data, version, wait)
        for marker in install.glob("runtime/self-update/*/helper.pid"):
            wait.wait_exit(int(marker.read_text()))
        assert hashlib.sha256(target.read_bytes()).hexdigest() == hashlib.sha256((BASE / f"binaries/{version}.exe").read_bytes()).hexdigest(), "Wrong executable installed"
        identity = json.loads(subprocess.check_output([str(target), "--build-identity"], timeout=15))
        assert identity["version"] == version
        assert_preserved(data, before)
        assert_preserved(external, outside)
        config = json.loads((data / "settings.json").read_text())
        assert config["apps"] == records[:1] and config["installations"] == records
        assert json.loads((data / "manager-settings.json").read_text())["theme"] == "light"
        hops.append(dict(version=version, executable_sha256=hashlib.sha256(target.read_bytes()).hexdigest(), restarted_pid=restarted["pid"], preserved_files=len(before) + len(outside)))
    return dict(case=name, folder=str(case), source=source, hops=hops, result="passed", settings_and_apps="byte-for-byte preserved", approval_refusal="passed", tamper_refusals=refusals)


def run():
    assert (BASE / ".craft-updater-fixture").read_text() == MARKER
    wait = wait_module()
    results = [run_case("direct", "1.1.0", ["1.2.0"], wait), run_case("bridge", "1.0.0", ["1.1.0", "1.2.0"], wait)]
    evidence = dict(worker=WORKER, host="Windows", results=results,
        scope="Actual HTTPS fixture downloads, signature/routing/hash/identity/commit checks and executable replacement/restart. Test-only artifact origin adapter and helper wrapper; not production-v2 wrapper or native/Linux/macOS evidence.")
    (BASE / "evidence.json").write_text(json.dumps(evidence, indent=2))
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "run"])
    args = parser.parse_args()
    (prepare if args.action == "prepare" else run)()
