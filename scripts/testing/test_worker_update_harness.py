"""Offline contract tests for the isolated live Worker transaction harness."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import importlib.util
from pathlib import Path
import tempfile
import unittest

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


if __name__ == "__main__":
    unittest.main()
