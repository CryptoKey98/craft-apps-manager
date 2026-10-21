"""Isolated DMG layout smoke test. Run with the packaging requirements installed."""

import importlib.util
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(sys.platform == "darwin", "Requires macOS disk-image tools")
@unittest.skipUnless(importlib.util.find_spec("dmgbuild"), "Install packaging/macos/dmg-requirements.txt")
class MacOSDmgTests(unittest.TestCase):
    def test_image_preserves_app_and_presents_drag_to_applications(self):
        from ds_store import DSStore

        with tempfile.TemporaryDirectory(prefix="craft dmg layout ") as folder:
            folder = Path(folder)
            bundle = folder / "Craft Apps Manager.app"
            (bundle / "Contents").mkdir(parents=True)
            marker = b"isolated layout fixture, not an executable"
            (bundle / "Contents/fixture").write_bytes(marker)
            image = folder / "layout.dmg"
            subprocess.run([
                sys.executable, "-m", "dmgbuild", "-s",
                str(ROOT / "packaging/macos/dmg-settings.py"),
                "-D", f"app={bundle}", "-D",
                f"background={ROOT / 'packaging/macos/dmg-background.tiff'}",
                "Craft Layout Test", str(image),
            ], check=True)
            subprocess.run(["hdiutil", "verify", str(image)], check=True, capture_output=True)
            mount = folder / "mounted"
            mount.mkdir()
            result = subprocess.check_output([
                "hdiutil", "attach", "-plist", "-readonly", "-nobrowse", "-noautoopen",
                "-mountpoint", str(mount), str(image),
            ])
            try:
                mounts = [entry["mount-point"] for entry in plistlib.loads(result)["system-entities"]
                          if "mount-point" in entry]
                self.assertIn(mount.resolve(), [Path(path).resolve() for path in mounts])
                self.assertEqual((mount / bundle.name / "Contents/fixture").read_bytes(), marker)
                self.assertTrue((mount / "Applications").is_symlink())
                self.assertEqual((mount / "Applications").readlink(), Path("/Applications"))
                self.assertFalse((mount / "portable.txt").exists())
                with DSStore.open(str(mount / ".DS_Store"), "r") as store:
                    self.assertEqual(store[bundle.name]["Iloc"], (140, 120))
                    self.assertEqual(store["Applications"]["Iloc"], (500, 120))
                    self.assertEqual(store["."]["icvp"]["backgroundType"], 2)
                    self.assertFalse(store["."]["bwsp"]["ShowToolbar"])
            finally:
                subprocess.run(["hdiutil", "detach", str(mount)], check=True)


if __name__ == "__main__":
    unittest.main()
