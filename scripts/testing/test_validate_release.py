
# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import importlib.util
from pathlib import Path
import re
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("validate_release", (project_root() / "scripts/publishing/validate-release.py"))
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


class ReleaseWorkflowTests(unittest.TestCase):
    def test_build_jobs_cover_tag_releases_and_manual_platform_selection(self):
        workflow = (project_root() / ".github/workflows/release.yml").read_text()
        conditions = dict(re.findall(r"^  (windows|linux|arch|macos):\n    if: (.+)$", workflow, re.MULTILINE))
        self.assertEqual(set(conditions), {"windows", "linux", "arch", "macos"})

        def selected(condition, event, platform, arch_enabled=True):
            # Evaluate only the equality/disjunction grammar used by build jobs.
            # Reject unexpected syntax rather than executing workflow text.
            enabled = True
            arch_guard = " && needs.validate.outputs.linux_arch == 'true'"
            if condition.endswith(arch_guard):
                condition = condition[:-len(arch_guard)]
                self.assertTrue(condition.startswith("(") and condition.endswith(")"))
                condition = condition[1:-1]
                enabled = arch_enabled
            comparisons = []
            for clause in condition.split(" || "):
                match = re.fullmatch(r"(github.event_name|inputs.platform) (==|!=) '([^']*)'", clause)
                self.assertIsNotNone(match, f"Unsupported job condition: {clause}")
                value = event if match[1] == "github.event_name" else platform
                comparisons.append((value == match[3]) if match[2] == "==" else (value != match[3]))
            return enabled and any(comparisons)

        for event, platform, expected in [
            ("push", "", {"windows", "linux", "arch", "macos"}),
            ("workflow_dispatch", "all", {"windows", "linux", "arch", "macos"}),
            ("workflow_dispatch", "windows", {"windows"}),
            ("workflow_dispatch", "linux", {"linux", "arch"}),
            ("workflow_dispatch", "macos", {"macos"}),
        ]:
            with self.subTest(event=event, platform=platform):
                actual = {job for job, condition in conditions.items() if selected(condition, event, platform)}
                self.assertEqual(actual, expected)
                without_arch = {job for job, condition in conditions.items() if selected(condition, event, platform, False)}
                self.assertEqual(without_arch, expected - {"arch"})


if __name__ == "__main__":
    unittest.main()
