import json
import hashlib
import os
import runpy
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat


class RevisionManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        (self.root / "assets").mkdir()
        self.script = self.root / "scripts/revision-manifest.py"
        shutil.copyfile(Path(__file__).with_name("revision-manifest.py"), self.script)
        self.key = Ed25519PrivateKey.from_private_bytes(bytes([42]) * 32)
        (self.root / "assets/update-public-key.hex").write_text(self.key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw).hex())
        self.env = dict(os.environ, CRAFT_UPDATE_SIGNING_KEY=(bytes([42]) * 32).hex())

    def run_manifest(self, folder, revision, platform="all", previous=None):
        args = [sys.executable, str(self.script), "--packages", str(folder), "--version", "0.8.3", "--repository", "owner/manager", "--revision", str(revision), "--platform", platform]
        if previous:
            args += ["--previous", str(previous)]
        return subprocess.run(args, env=self.env, capture_output=True, text=True)

    def read_manifest(self, folder):
        envelope = json.loads((folder / "updates-v1.json").read_text())
        self.key.public_key().verify(bytes.fromhex(envelope["signature"]), envelope["payload"].encode())
        return json.loads(envelope["payload"])

    def baseline(self):
        folder = self.root / "baseline"
        folder.mkdir()
        for name in ["Craft-Apps-Manager-0.8.3-windows-x64.zip", "Craft-Apps-Manager-0.8.3-linux-x64.zip", "Craft-Apps-Manager-0.8.3-macos-universal.zip"]:
            (folder / name).write_bytes(b"revision 1 fixture")
        result = self.run_manifest(folder, 1)
        self.assertEqual(result.returncode, 0, result.stderr)
        return folder

    def test_baseline_keeps_legacy_downloads_and_creates_signed_immutable_copies(self):
        folder = self.baseline()
        manifest = self.read_manifest(folder)
        self.assertEqual(len(manifest["packages"]), 3)
        for package in manifest["packages"]:
            self.assertEqual(package["revision"], 1)
            self.assertEqual(package["asset"], f"Craft Apps Manager-0.8.3-{package['platform']}-{package['architecture']}.zip")
            self.assertNotIn("Rev", package["asset"])
            self.assertTrue((folder / package["asset"]).exists())
        self.assertTrue((folder / "Craft-Apps-Manager-0.8.3-linux-x64.zip").exists())

    def test_linux_hotfix_preserves_other_targets_and_refreshes_legacy_download(self):
        baseline = self.baseline()
        folder = self.root / "hotfix"
        folder.mkdir()
        legacy = folder / "Craft-Apps-Manager-0.8.3-linux-x64.zip"
        legacy.write_bytes(b"revision 2 fixture")
        result = self.run_manifest(folder, 2, "linux", baseline / "updates-v1.json")
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = self.read_manifest(folder)
        revisions = {p["platform"]: p["revision"] for p in manifest["packages"]}
        self.assertEqual(revisions, {"windows": 1, "linux": 2, "macos": 1})
        self.assertEqual(legacy.read_bytes(), b"revision 2 fixture")
        self.assertEqual((baseline / "Craft Apps Manager-0.8.3-linux-x64.zip").read_bytes(), b"revision 1 fixture")
        linux = next(p for p in manifest["packages"] if p["platform"] == "linux")
        self.assertEqual(linux["asset"], "Craft Apps Manager-0.8.3-Rev-2-linux-x64.zip")

    def test_revision_names_cover_all_package_formats(self):
        name = runpy.run_path(str(self.script))["revision_name"]
        for platform, architecture, kind, extension in [
            ("windows", "x64", "zip", ".zip"),
            ("windows", "x86", "msi", ".msi"),
            ("macos", "universal", "zip", ".zip"),
            ("linux", "x64", "deb", ".deb"),
            ("linux", "x86", "rpm", ".rpm"),
            ("linux", "x64", "arch", ".pkg.tar.zst"),
        ]:
            self.assertEqual(name("0.8.3", 1, platform, architecture, kind), f"Craft Apps Manager-0.8.3-{platform}-{architecture}{extension}")
            self.assertEqual(name("0.8.3", 2, platform, architecture, kind), f"Craft Apps Manager-0.8.3-Rev-2-{platform}-{architecture}{extension}")

    def test_nonadvancing_revision_and_tampered_previous_manifest_are_rejected(self):
        baseline = self.baseline()
        folder = self.root / "bad"
        folder.mkdir()
        (folder / "Craft-Apps-Manager-0.8.3-linux-x64.zip").write_bytes(b"fixture")
        self.assertNotEqual(self.run_manifest(folder, 1, "linux", baseline / "updates-v1.json").returncode, 0)
        previous = json.loads((baseline / "updates-v1.json").read_text())
        previous["payload"] += " "
        (baseline / "updates-v1.json").write_text(json.dumps(previous))
        self.assertNotEqual(self.run_manifest(folder, 2, "linux", baseline / "updates-v1.json").returncode, 0)
        self.assertFalse((folder / "updates-v1.json").exists())

    def test_hotfix_rejects_unrelated_platform_packages(self):
        baseline = self.baseline()
        folder = self.root / "wrong-platform"
        folder.mkdir()
        (folder / "Craft-Apps-Manager-0.8.3-windows-x64.zip").write_bytes(b"fixture")
        self.assertNotEqual(self.run_manifest(folder, 2, "linux", baseline / "updates-v1.json").returncode, 0)

    @unittest.skipUnless(shutil.which("dpkg-deb"), "Debian package tools are required")
    def test_debian_legacy_download_contains_latest_payload_with_compatible_metadata(self):
        baseline = self.baseline()
        folder = self.root / "debian-hotfix"
        folder.mkdir()
        staging = self.root / "debian-staging"
        (staging / "DEBIAN").mkdir(parents=True)
        (staging / "usr/bin").mkdir(parents=True)
        (staging / "DEBIAN/control").write_text(
            "Package: craft-apps-manager\nVersion: 0.8.3-2\nArchitecture: amd64\n"
            "Maintainer: Test <test@example.com>\nDescription: Revision fixture\n"
        )
        payload = b"revision 2 executable fixture"
        (staging / "usr/bin/craft-apps-manager").write_bytes(payload)
        package = folder / "craft-apps-manager_0.8.3-2_amd64.deb"
        subprocess.run(["dpkg-deb", "--build", "--root-owner-group", str(staging), str(package)], check=True, capture_output=True)
        result = self.run_manifest(folder, 2, "linux", baseline / "updates-v1.json")
        self.assertEqual(result.returncode, 0, result.stderr)
        entry = next(p for p in self.read_manifest(folder)["packages"] if p["format"] == "deb")
        canonical = folder / entry["asset"]
        self.assertEqual(entry["package_version"], "0.8.3-2")
        self.assertEqual(entry["sha256"], hashlib.sha256(canonical.read_bytes()).hexdigest())
        compatibility = folder / "craft-apps-manager_0.8.3_amd64.deb"
        for label, file, expected in [("canonical", canonical, "0.8.3-2"), ("legacy", compatibility, "0.8.3")]:
            version = subprocess.check_output(["dpkg-deb", "-f", str(file), "Version"], text=True).strip()
            self.assertEqual(version, expected)
            destination = self.root / label
            subprocess.run(["dpkg-deb", "-x", str(file), str(destination)], check=True)
            self.assertEqual((destination / "usr/bin/craft-apps-manager").read_bytes(), payload)


if __name__ == "__main__":
    unittest.main()
