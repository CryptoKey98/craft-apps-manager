"""Read-only entrypoint and repository-root contracts for organized scripts."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))


class ScriptEntrypointsTests(unittest.TestCase):
    def test_direct_shell_workflow_entrypoints_keep_executable_git_mode(self):
        if not (ROOT / '.git').exists():
            self.skipTest('Executable Git modes require a repository checkout')
        paths=set()
        for workflow in (ROOT / '.github/workflows').glob('*.yml'):
            paths.update(re.findall(r"\./(scripts/[^\s'\"]+\.sh)",workflow.read_text(encoding='utf-8')))
        self.assertTrue(paths, 'No direct shell entrypoints found')
        for path in sorted(paths):
            entry=subprocess.check_output(['git','ls-files','--stage','--',path],cwd=ROOT,text=True)
            self.assertTrue(entry, f'Untracked workflow entrypoint: {path}')
            self.assertEqual(entry.split()[0], '100755', f'Workflow directly executes non-executable script: {path}')

    def test_catalog_workflow_imports_resolve_without_running_publication(self):
        workflow = (ROOT / ".github/workflows/renew-update-catalog.yml").read_text(encoding="utf-8")
        imports = re.findall(r"(?m)^ +from (?:lib\.)?updater_config import [^\n]+", workflow)
        self.assertEqual(len(imports), 2, "Check both signing and deployment imports")
        for statement in imports:
            code = ("import sys; sys.path.insert(0, sys.argv[1]); " + statement.strip())
            result = subprocess.run([sys.executable, "-c", code, str(ROOT / "scripts")],
                                    cwd=ROOT, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def canonical(self, relative):
        path = ROOT / "scripts" / relative
        self.assertTrue(path.is_file(), f"Missing canonical script: {relative}")
        return path

    def run_script(self, path, *args, cwd=None, env=None):
        return subprocess.run([sys.executable, str(path), *args], cwd=cwd or ROOT,
                              env=env, text=True, capture_output=True)

    def test_publishing_commands_are_directly_executable(self):
        for name in ("revision-manifest.py", "stage-v2-assets.py", "immutable-assets.py",
                     "update-catalog.py", "validate-release.py"):
            result = self.run_script(self.canonical("publishing/" + name), "--help")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("usage:", result.stdout)

    def test_project_root_is_independent_of_cwd(self):
        self.canonical("lib/repository.py")
        target = ROOT / "target/reorganization-checks"
        target.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="unrelated cwd ", dir=target) as folder:
            code = ("import sys; sys.path.insert(0, sys.argv[1]); "
                    "from lib.repository import project_root; print(project_root())")
            result = subprocess.run([sys.executable, "-c", code, str(ROOT / "scripts")],
                                    cwd=folder, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(Path(result.stdout.strip()), ROOT)

    def test_test_command_discovers_cases(self):
        result = self.run_script(self.canonical("testing/test_validate_release.py"))
        self.assertEqual(result.returncode, 0, result.stderr)
        count = int(re.search(r"Ran (\d+) tests?", result.stderr)[1])
        self.assertGreater(count, 0)

    def test_fork_config_with_spaces_keeps_scope_and_key(self):
        canonical = self.canonical("lib/updater_config.py")
        target = ROOT / "target/reorganization-checks"
        target.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="fork configuration ", dir=target) as folder:
            folder = Path(folder)
            config = folder / "fork updater.toml"
            (folder / "key.hex").write_text("07" * 32, encoding="utf-8")
            config.write_text("repository='fork/manager'\npublic_key_file='key.hex'\n"
                              "[revisions]\nwindows=2\nlinux=3\nmacos=4\n", encoding="utf-8")
            env = {k: v for k, v in os.environ.items()
                   if not k.startswith(("CRAFT_UPDATE", "CRAFT_RELEASE"))
                   and k not in {"CRAFT_MANAGER_REPOSITORY", "CRAFT_BUILD_REVISION"}}
            args = ("export", "--target-os", "linux", "--config", str(config))
            new = self.run_script(canonical, *args, cwd=folder, env=env)
            self.assertEqual(new.returncode, 0, new.stderr)
            value = json.loads(new.stdout)
            self.assertEqual(value["repository"], "fork/manager")
            self.assertEqual(value["revision"], 3)
            self.assertEqual(value["source"], "github")
            self.assertEqual(value["public_key_hex"], "07" * 32)
            for bad in (folder / "missing.toml", folder / "invalid.toml"):
                if bad.name == "invalid.toml":
                    bad.write_text("not valid [", encoding="utf-8")
                args = ("export", "--target-os", "linux", "--config", str(bad))
                new = self.run_script(canonical, *args, cwd=folder, env=env)
                self.assertNotEqual(new.returncode, 0)

    def test_linux_zip_preserves_guide_and_readme_navigation(self):
        import zipfile
        script = self.canonical("packaging/package-linux-zip.sh").read_text(encoding="utf-8")
        payload_writer = script.split("<<'PY'\n", 1)[1].split("\nPY", 1)[0]
        target = ROOT / "target/reorganization-checks"
        target.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="linux package docs ", dir=target) as folder:
            folder = Path(folder)
            fixture = folder / "source"
            for relative in ("README.md", "docs/user-guide/linux.md", "CHANGELOG.md",
                             "LICENSE", "THIRD-PARTY-NOTICES.txt", "assets/icon.png",
                             "assets/fonts/OFL.txt", "assets/app-icons/license.txt"):
                output = fixture / relative
                output.parent.mkdir(parents=True, exist_ok=True)
                source = ROOT / relative
                output.write_bytes(source.read_bytes() if source.exists() else b"fixture license")
            binary = folder / "manager"
            binary.write_bytes(b"fixture executable")
            archive = folder / "manager.zip"
            result = subprocess.run([sys.executable, "-c", payload_writer, str(fixture),
                                     str(binary), str(archive)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            prefix = "Craft Apps Manager Linux/"
            with zipfile.ZipFile(archive) as package:
                readme = package.read(prefix + "README.md").decode("utf-8")
                link = re.search(r"\[Linux setup\]\(([^)]+)\)", readme)[1]
                self.assertIn(prefix + link, package.namelist())
                guide = (ROOT / "docs/user-guide/linux.md").read_bytes()
                self.assertEqual(package.read(prefix + link), guide)
                self.assertEqual(package.read(prefix + "docs/linux.md"), guide)



if __name__ == "__main__":
    unittest.main()
