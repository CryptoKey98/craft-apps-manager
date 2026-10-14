import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("validate_release", Path(__file__).with_name("validate-release.py"))
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


class ReleaseValidationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.files = {
            "Cargo.toml": '[package]\nname = "craft-apps-manager"\nversion = "0.8.2"\n[dependencies]\n',
            "Cargo.lock": '[[package]]\nname = "craft-apps-manager"\nversion = "0.8.2"\n[[package]]\nname = "dependency"\nversion = "0.8.1"\n',
            "README.md": "Version **0.8.2** includes fixes\n",
            "CHANGELOG.md": "# Changelog\n\n## 0.8.2\n\n- Fix cache cleanup.\n\n## 0.8.1\n\n- Older change.\n",
        }
        self.write()

    def write(self):
        for name, text in self.files.items():
            (self.root / name).write_text(text, encoding="utf-8")

    def test_extracts_only_current_notes_with_lf_and_crlf(self):
        for newline in ("\n", "\r\n"):
            (self.root / "CHANGELOG.md").write_bytes(self.files["CHANGELOG.md"].replace("\n", newline).encode())
            self.assertEqual(validator.release_notes(self.root, "v0.8.2"), "- Fix cache cleanup.\n")

    def test_rejects_stale_references(self):
        for name in self.files:
            with self.subTest(name=name):
                self.write()
                (self.root / name).write_text(self.files[name].replace("0.8.2", "0.8.1"), encoding="utf-8")
                with self.assertRaises(ValueError):
                    validator.release_notes(self.root, "v0.8.2")

    def test_rejects_invalid_tags(self):
        for tag in ("0.8.2", "v0.8.2-beta", "v256.0.0", "v0.0.65536", "v00.8.2"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                validator.release_notes(self.root, tag)

    def test_rejects_missing_notes_and_placeholders(self):
        for notes in ("", "Describe the changes before merging this release preparation."):
            (self.root / "CHANGELOG.md").write_text(f"# Changelog\n\n## 0.8.2\n\n{notes}\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                validator.release_notes(self.root, "v0.8.2")


if __name__ == "__main__":
    unittest.main()
