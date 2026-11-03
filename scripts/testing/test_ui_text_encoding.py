"""Catch accidental legacy-codepage decoding of UTF-8 UI text."""
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]


class UiTextEncodingTests(unittest.TestCase):
    def test_ui_sources_have_no_mojibake_markers(self):
        markers = ("\u00c3", "\u00c2", "\u00e2\u20ac", "\ufffd")
        failures = []
        for path in sorted((ROOT / "src" / "ui").rglob("*.rs")):
            text = path.read_bytes().decode("utf-8")
            for number, line in enumerate(text.splitlines(), 1):
                if any(marker in line for marker in markers):
                    failures.append(f"{path.relative_to(ROOT)}:{number}")
        self.assertEqual(failures, [], "Suspect UI text encoding at: " + ", ".join(failures))


if __name__ == "__main__":
    unittest.main()
