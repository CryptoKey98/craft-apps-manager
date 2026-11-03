"""Offline contract tests for the isolated live Worker transaction harness."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

ROOT = project_root()


class HarnessTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("worker_harness", ROOT / "scripts/testing/test-worker-updates.py")
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)

    def test_mutated_app_file_fails_preservation_check(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            app = root / "apps/Fixture App/app.exe"
            app.parent.mkdir(parents=True)
            app.write_bytes(b"fixture application")
            before = self.module.snapshot(root)
            self.module.assert_preserved(root, before)
            app.write_bytes(b"overwritten")
            with self.assertRaises(AssertionError):
                self.module.assert_preserved(root, before)

    def test_windows_bridge_requires_but_direct_skips_migration_hop(self):
        catalog = self.module.windows_records({"1.1.0": (12, "a" * 64), "1.2.0": (13, "b" * 64)})
        self.assertEqual(len(catalog["packages"]), 2)
        by_id = {route["id"]: route for route in catalog["routes"]}
        self.assertTrue(by_id["windows-to-b"]["bridge"])
        self.assertEqual(by_id["windows-to-c"]["min_source"]["version"], "1.1.0")
        self.assertTrue(all(p["handler"] == "archive.zip.v1" for p in catalog["packages"]))

    def run_migrated_case(self, folder, theme="light"):
        base = Path(folder)
        (base / "binaries").mkdir()
        for version in ("1.0.0", "1.1.0", "1.2.0"):
            (base / "binaries" / (version + ".exe")).write_bytes(version.encode())
        stages = []
        updates = []

        def run(args, **kwargs):
            if "--apply-self-update" in args:
                return SimpleNamespace(returncode=1, stderr=b"tampered fixture rejected")
            data = Path(args[args.index("--root") + 1])
            if "--worker" not in args:
                # Simulate production startup migration before snapshotting data.
                legacy = data / "manager-settings.json"
                (data / "config.yaml").write_text(json.dumps({"schema_version": 1, "manager": json.loads(legacy.read_text())}))
                legacy.unlink()
                (data / "worker-fixture-receipt.json").write_text("{}")
            elif "--approve" not in args:
                return SimpleNamespace(returncode=1, stderr=b"explicit approval required")
            else:
                version = "1.2.0" if updates else "1.1.0"
                updates.append(version)
                Path(args[0]).write_bytes((base / "binaries" / (version + ".exe")).read_bytes())
            return SimpleNamespace(returncode=0, stderr=b"")

        def output(args, **kwargs):
            if "--build-identity" in args:
                return json.dumps({"version": updates[-1]}).encode()
            stage = base / ("stage-" + str(len(stages)))
            stage.mkdir()
            stages.append(stage)
            (stage / "fixture-authorization.json").write_text('{"source_hash":"original"}')
            (stage / "download.package").write_bytes(b"package fixture")
            return str(stage / "plan.json").encode()

        def restarted(data, version, wait):
            self.assertTrue((data / "config.yaml").is_file())
            self.assertFalse((data / "manager-settings.json").exists())
            return {"version": version, "pid": 0, "preferences": {"theme": theme}}

        with patch.object(self.module, "BASE", base), \
                patch.object(self.module.subprocess, "run", side_effect=run), \
                patch.object(self.module.subprocess, "check_output", side_effect=output), \
                patch.object(self.module, "receipt", side_effect=restarted):
            return self.module.run_case("migrated", "1.0.0", ["1.1.0", "1.2.0"], SimpleNamespace(wait_exit=lambda pid: None))

    def test_migrated_yaml_preferences_allow_all_bridge_hops_to_verify(self):
        with tempfile.TemporaryDirectory() as folder:
            result = self.run_migrated_case(folder)
            self.assertEqual(result["result"], "passed")
            self.assertEqual([hop["version"] for hop in result["hops"]], ["1.1.0", "1.2.0"])

    def test_changed_typed_preference_still_fails_verification(self):
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaisesRegex(AssertionError, "preference"):
                self.run_migrated_case(folder, theme="dark")


if __name__ == "__main__":
    unittest.main()
