"""Local-only wiki generation and reconciliation contracts; never publish."""
import importlib.util
import json
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts/documentation'))


class WikiTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import wiki
        cls.wiki = wiki
        cls.manifest = wiki.load_manifest(ROOT / 'scripts/documentation/wiki-manifest.json')

    def setUp(self):
        folder = ROOT / 'target/wiki-sync'
        folder.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=folder)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        manifest_path = self.root / 'scripts/documentation/wiki-manifest.json'
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(json.dumps(self.manifest), encoding='utf-8')
        for item in self.manifest['pages'] + self.manifest['assets']:
            path = self.root / item['source']
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'# Example\n\nPublic text.\n' if path.suffix == '.md' else b'png fixture')
        for name in ('LOCAL-NOTES.md', 'docs/private/notes.md', '.env', '.release-secrets/key.hex', 'docs/unlisted.md'):
            p = self.root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text('DO_NOT_EXPORT_SENTINEL', encoding='utf-8')

    def prepare(self, manifest=None):
        return self.wiki.prepare(self.root, manifest or self.manifest, 'a' * 40, self.root / 'target/wiki-preview')

    def test_preview_is_allowlisted_deterministic_and_preserves_sources(self):
        before = {p: p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        files = self.prepare()
        self.assertEqual(len(files), 26)
        self.assertEqual(len([p for p in files if p.endswith('.md')]), 17)
        self.assertFalse(any(b'DO_NOT_EXPORT_SENTINEL' in content for content in files.values()))
        self.assertEqual(files, self.prepare())
        for p, body in before.items():
            self.assertEqual(p.read_bytes(), body)

    def test_invalid_manifest_and_output_preserve_siblings(self):
        sentinel = self.root / 'target/release/keep.txt'
        sentinel.parent.mkdir(parents=True)
        sentinel.write_text('keep')
        for source in ('../escape.md', '/absolute.md', 'docs/missing.md', 'LOCAL-NOTES.md'):
            invalid = json.loads(json.dumps(self.manifest))
            invalid['pages'][0]['source'] = source
            with self.subTest(source=source), self.assertRaises(ValueError):
                self.prepare(invalid)
        invalid = json.loads(json.dumps(self.manifest))
        invalid['pages'][1]['destination'] = 'home.MD'
        with self.assertRaises(ValueError):
            self.prepare(invalid)
        with self.assertRaises(ValueError):
            self.wiki.prepare(self.root, self.manifest, 'a' * 40, self.root / 'target/release')
        self.assertEqual(sentinel.read_text(), 'keep')

    def test_links_images_references_and_code_are_preserved(self):
        text = '''# Example
[Start](../README.md#get-started)
[Nested [label]](user-guide/linux.md "Linux guide")
![Screenshot](images/main.png?v=0.8.0)
<img src="../assets/icon.png" width="80" alt="Icon">
[Legacy][old]
[old]: linux.md#folders "old title"
`[Code](missing.md)`
~~~sh
[Command](also-missing.md)
~~~
<details><summary>More</summary>
[License](../LICENSE)
</details>
'''
        result = self.wiki.rewrite_markdown(text, 'docs/README.md', self.manifest, 'a' * 40)
        self.assertIn('/wiki/Getting-started#get-started', result)
        self.assertIn('/wiki/Linux "Linux guide"', result)
        self.assertIn('raw.githubusercontent.com/wiki/CryptoKey98/craft-apps-manager/images/main.png?v=0.8.0', result)
        self.assertIn('images/icon.png" width="80" alt="Icon"', result)
        self.assertIn('/wiki/Linux#folders "old title"', result)
        self.assertIn('`[Code](missing.md)`', result)
        self.assertIn('[Command](also-missing.md)', result)
        self.assertIn('/blob/' + 'a' * 40 + '/LICENSE', result)
        self.assertIn('<details><summary>More</summary>', result)
        for bad in ('[Missing](missing.md)', '[Escape](../../secret.md)', '![Bad](images/unknown.png)'):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                self.wiki.rewrite_markdown(bad, 'docs/README.md', self.manifest, 'a' * 40)

    def test_real_repo_links_and_generated_navigation_validate(self):
        files = self.wiki.generate(ROOT, self.manifest, 'a' * 40)
        self.wiki.validate_preview(files, self.manifest)
        for page in self.manifest['pages']:
            self.assertEqual(files['_Sidebar.md'].decode().count(self.wiki.page_url(page['destination']) + ')'), 1)
            self.assertIn('/' + page['source'], files[page['destination']].decode())
        damaged = dict(files)
        damaged['Home.md'] += b'\n[Broken](https://github.com/CryptoKey98/craft-apps-manager/wiki/Linux#missing-heading)\n'
        with self.assertRaises(ValueError):
            self.wiki.validate_preview(damaged, self.manifest)

    def test_owned_sync_preserves_manual_pages_and_stops_on_edits(self):
        import wiki_sync
        files = self.prepare()
        checkout = self.root / 'wiki'
        checkout.mkdir()
        (checkout / 'Notes.md').write_text('Human notes')
        (checkout / 'Home.md').write_text('Existing Home')
        blocked = wiki_sync.plan_changes(checkout, files, None, set())
        self.assertTrue(blocked['conflicts'])
        with self.assertRaises(ValueError):
            wiki_sync.apply_changes(checkout, blocked)
        plan = wiki_sync.plan_changes(checkout, files, None, {'Home.md'})
        self.assertFalse(plan['conflicts'])
        wiki_sync.apply_changes(checkout, plan)
        previous = json.loads((checkout / self.wiki.OWNERSHIP).read_text())
        noop = wiki_sync.plan_changes(checkout, files, previous, set())
        self.assertEqual(noop['writes'], {})
        self.assertEqual(noop['deletes'], [])
        (checkout / 'Home.md').write_text('Manual correction')
        blocked = wiki_sync.plan_changes(checkout, files, previous, set())
        self.assertTrue(blocked['conflicts'])
        self.assertEqual((checkout / 'Notes.md').read_text(), 'Human notes')
        with self.assertRaises(ValueError):
            wiki_sync.plan_changes(checkout, files, None, set())

    def test_sync_removes_only_unchanged_owned_files(self):
        import wiki_sync
        files = self.prepare()
        checkout = self.root / 'wiki'
        checkout.mkdir()
        wiki_sync.apply_changes(checkout, wiki_sync.plan_changes(checkout, files, None, set()))
        previous = json.loads((checkout / self.wiki.OWNERSHIP).read_text())
        (checkout / 'Notes.md').write_text('keep')
        changed = dict(files)
        changed['Renamed.md'] = changed.pop('Linux.md')
        changed[self.wiki.OWNERSHIP] = self.wiki.ownership({k: v for k, v in changed.items() if k != self.wiki.OWNERSHIP}, 'b' * 40)
        plan = wiki_sync.plan_changes(checkout, changed, previous, set())
        self.assertEqual(plan['deletes'], ['Linux.md'])
        wiki_sync.apply_changes(checkout, plan)
        self.assertTrue((checkout / 'Renamed.md').is_file())
        self.assertFalse((checkout / 'Linux.md').exists())
        self.assertEqual((checkout / 'Notes.md').read_text(), 'keep')

    def test_metadata_paths_and_partial_previews_fail_before_mutation(self):
        files = self.prepare()
        preview = self.root / 'target/wiki-preview'
        prior = json.loads((preview / self.wiki.OWNERSHIP).read_text())
        prior['files']['../release/keep.txt'] = 'a' * 64
        (preview / self.wiki.OWNERSHIP).write_text(json.dumps(prior))
        (preview / 'Unowned.md').write_text('keep')
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertEqual((preview / 'Unowned.md').read_text(), 'keep')

    def test_local_git_roundtrip_and_non_fast_forward_rejection(self):
        import wiki_sync
        env = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT='0')
        def git(cwd, *args, check=True):
            return subprocess.run(['git', '-c', 'core.autocrlf=false', '-c', 'init.templateDir=', *args],
                                  cwd=cwd, env=env, capture_output=True, text=True, check=check)
        remote = self.root / 'remote.git'
        git(self.root, 'init', '--bare', '--initial-branch=wiki-fixture', str(remote))
        first = self.root / 'first'
        git(self.root, 'clone', str(remote), str(first))
        def identity(folder):
            git(folder, 'config', 'user.name', 'Local Wiki Fixture')
            git(folder, 'config', 'user.email', 'wiki-fixture@example.invalid')
        identity(first)
        (first / 'Notes.md').write_text('Human notes')
        files = self.prepare()
        def apply(folder, output):
            record = folder / self.wiki.OWNERSHIP
            previous = json.loads(record.read_text()) if record.exists() else None
            plan = wiki_sync.plan_changes(folder, output, previous, set())
            wiki_sync.apply_changes(folder, plan)
            if not plan['writes'] and not plan['deletes']:
                return False
            git(folder, 'add', '--', *sorted(set(plan['writes']) | set(plan['deletes'])))
            git(folder, 'commit', '-m', 'Sync local documentation fixture')
            return True
        self.assertTrue(apply(first, files))
        git(first, 'add', '--', 'Notes.md')
        git(first, 'commit', '-m', 'Add manual fixture note')
        git(first, 'push', 'origin', 'HEAD:refs/heads/wiki-fixture')
        initial = git(first, 'rev-parse', 'HEAD').stdout
        self.assertFalse(apply(first, files))
        self.assertEqual(git(first, 'rev-parse', 'HEAD').stdout, initial)
        second = self.root / 'second'
        git(self.root, 'clone', str(remote), str(second))
        identity(second)
        self.assertEqual(git(second, 'branch', '--show-current').stdout.strip(), 'wiki-fixture')
        (self.root / 'docs/README.md').write_text('# Changed home\n\nUpdated source.\n')
        updated = self.wiki.generate(self.root, self.manifest, 'b' * 40)
        self.assertTrue(apply(first, updated))
        (second / 'Notes.md').write_text('Concurrent human edit')
        git(second, 'add', '--', 'Notes.md')
        git(second, 'commit', '-m', 'Concurrent note')
        git(second, 'push', 'origin', 'HEAD:refs/heads/wiki-fixture')
        result = git(first, 'push', 'origin', 'HEAD:refs/heads/wiki-fixture', check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('rejected', result.stderr)
        self.assertEqual(git(self.root, '--git-dir=' + str(remote), 'rev-parse', 'refs/heads/wiki-fixture').stdout,
                         git(second, 'rev-parse', 'HEAD').stdout)
        (first / 'Home.md').write_text('Manual generated edit')
        previous = json.loads((first / self.wiki.OWNERSHIP).read_text())
        self.assertTrue(wiki_sync.plan_changes(first, updated, previous, set())['conflicts'])

    def test_symlinks_and_changed_plan_are_rejected(self):
        import wiki_sync
        files = self.prepare()
        checkout = self.root / 'wiki'
        checkout.mkdir()
        plan = wiki_sync.plan_changes(checkout, files, None, set())
        (checkout / 'Home.md').write_text('Added after planning')
        with self.assertRaises(ValueError):
            wiki_sync.apply_changes(checkout, plan)
        source = self.root / 'docs/README.md'
        source.unlink()
        try:
            source.symlink_to(self.root / 'README.md')
        except OSError:
            self.skipTest('This Windows session cannot create symlinks')
        with self.assertRaises(ValueError):
            self.prepare()

    def test_anchors_include_inline_code_and_duplicate_headings(self):
        self.assertEqual(self.wiki.anchors('# Use `portable.txt`\n## Repeat\n## Repeat\n'),
                         {'use-portabletxt', 'repeat', 'repeat-1'})
        self.assertEqual(self.wiki.anchors('# Repeat\n# Repeat-1\n# Repeat\n'),
                         {'repeat', 'repeat-1', 'repeat-2'})

    def test_preview_refuses_unlisted_output_without_deleting_it(self):
        self.prepare()
        extra = self.root / 'target/wiki-preview/LOCAL-NOTES.md'
        extra.write_text('PRIVATE_OUTPUT_SENTINEL')
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertEqual(extra.read_text(), 'PRIVATE_OUTPUT_SENTINEL')

    def test_case_only_reconciliation_collision_is_rejected(self):
        import wiki_sync
        files = self.prepare()
        checkout = self.root / 'wiki'
        checkout.mkdir()
        wiki_sync.apply_changes(checkout, wiki_sync.plan_changes(checkout, files, None, set()))
        previous = json.loads((checkout / self.wiki.OWNERSHIP).read_text())
        changed = dict(files)
        changed['linux.md'] = changed.pop('Linux.md')
        changed[self.wiki.OWNERSHIP] = self.wiki.ownership({k: v for k, v in changed.items() if k != self.wiki.OWNERSHIP}, 'b' * 40)
        with self.assertRaises(ValueError):
            wiki_sync.plan_changes(checkout, changed, previous, set())

    def test_unknown_input_never_enters_real_output(self):
        files = self.wiki.generate(ROOT, self.manifest, 'a' * 40)
        expected = {p['destination'] for p in self.manifest['pages'] + self.manifest['assets']}
        expected.update({'_Sidebar.md', '_Footer.md', self.wiki.OWNERSHIP})
        self.assertEqual(set(files), expected)

    def test_publication_requires_enabled_main_event(self):
        import wiki_publish
        valid = {'GITHUB_ACTIONS': 'true', 'GITHUB_REPOSITORY': self.wiki.REPOSITORY,
                 'GITHUB_REF': 'refs/heads/main', 'GITHUB_EVENT_NAME': 'push',
                 'GITHUB_SHA': 'a' * 40, 'WIKI_SYNC_ENABLED': 'true', 'WIKI_SYNC_TOKEN': 'fixture-secret'}
        self.assertEqual(wiki_publish.authorize(valid), 'a' * 40)
        for key, value in [('GITHUB_ACTIONS', 'false'), ('GITHUB_REPOSITORY', 'other/repo'),
                           ('GITHUB_REF', 'refs/heads/feature'), ('GITHUB_EVENT_NAME', 'pull_request'),
                           ('GITHUB_SHA', 'bad'), ('WIKI_SYNC_ENABLED', 'false'), ('WIKI_SYNC_TOKEN', '')]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                wiki_publish.authorize(dict(valid, **{key: value}))

    def test_publication_checks_freshness_before_writes_and_push(self):
        import wiki_publish
        checkout = self.root / 'publish'
        checkout.mkdir()
        calls, stages = [], []
        def git(folder, *args):
            calls.append(args)
            return ''
        def fresh():
            stages.append('fresh')
            if len(stages) == 2:
                raise ValueError('Source advanced')
        with self.assertRaisesRegex(ValueError, 'Source advanced'):
            wiki_publish.sync_checkout(self.root, checkout, 'a' * 40, git, fresh)
        self.assertFalse(any(call[0] == 'push' for call in calls))
        stages.clear()
        checkout = self.root / 'publish-success'
        checkout.mkdir()
        self.assertTrue(wiki_publish.sync_checkout(self.root, checkout, 'a' * 40, git, lambda: None))
        self.assertTrue(any(call[0] == 'push' for call in calls))
        calls.clear()
        self.assertFalse(wiki_publish.sync_checkout(self.root, checkout, 'a' * 40, git, lambda: None))
        self.assertEqual(calls, [])

    def test_bootstrap_adopts_only_exact_home(self):
        import wiki_publish
        checkout = self.root / 'publish'
        checkout.mkdir()
        home = checkout / 'Home.md'
        home.write_bytes(b'Documentation setup in progress.\n')
        expected = hashlib.sha256(home.read_bytes()).hexdigest()
        with self.assertRaises(ValueError):
            wiki_publish.sync_checkout(self.root, checkout, 'a' * 40, lambda *args: '', lambda: None, 'b' * 64)
        self.assertEqual(home.read_bytes(), b'Documentation setup in progress.\n')
        self.assertTrue(wiki_publish.sync_checkout(self.root, checkout, 'a' * 40, lambda *args: '', lambda: None, expected))

    def test_git_failure_does_not_report_credential_output(self):
        import wiki_publish
        result = subprocess.CompletedProcess(['git'], 1, 'fixture-secret', 'remote echoed fixture-secret')
        with patch.object(wiki_publish.subprocess, 'run', return_value=result) as run:
            with self.assertRaises(ValueError) as caught:
                wiki_publish.git_runner({'WIKI_SYNC_TOKEN': 'fixture-secret'})(self.root, 'push', 'origin', 'HEAD')
            self.assertNotIn('fixture-secret', str(caught.exception))
            self.assertFalse(any('fixture-secret' in arg for arg in run.call_args.args[0]))
            self.assertTrue(run.call_args.kwargs['capture_output'])

    def test_docs_workflow_separates_validation_and_publication(self):
        text = (ROOT / '.github/workflows/docs.yml').read_text(encoding='utf-8')
        self.assertIn('contents: read', text)
        self.assertIn('persist-credentials: false', text)
        self.assertIn('pull_request:', text)
        self.assertIn('workflow_dispatch:', text)
        self.assertIn('python scripts/testing/test_wiki_documentation.py', text)
        self.assertIn('wiki.py check --source-commit', text)
        for source_family in ('docs/*.md', 'docs/user-guide/**', 'docs/development/**',
                              'docs/architecture/**', 'docs/images/**'):
            self.assertIn(source_family, text)
        self.assertIn('[ubuntu-latest, windows-latest]', text)
        validation = text.split('  wiki-publish:', 1)[0]
        self.assertNotIn('secrets.', validation)
        self.assertIn("github.ref == 'refs/heads/main'", text)
        self.assertIn("vars.WIKI_SYNC_ENABLED == 'true'", text)
        self.assertIn('environment: wiki-publication', text)
        self.assertIn('cancel-in-progress: false', text)
        self.assertIn('needs: wiki-preview', text)
        self.assertIn('secrets.WIKI_SYNC_TOKEN', text)
        for forbidden in ('contents: write', 'pull_request_target', 'deploy-pages',
                          'wrangler', 'release.yml', 'cargo '):
            self.assertNotIn(forbidden, text)

    @unittest.skipUnless(os.name == 'nt', 'Windows junction contract')
    def test_windows_junction_parent_is_not_followed(self):
        real = self.root / 'real'
        real.mkdir()
        (real / 'README.md').write_text('# Linked source')
        source_dir = self.root / 'docs/development'
        # Use a fresh alias to an existing source parent rather than deleting fixtures.
        junction = self.root / 'docs/linked'
        result = subprocess.run(['cmd', '/c', 'mklink', '/J', str(junction), str(real)], capture_output=True)
        if result.returncode:
            self.skipTest('Cannot create test junction')
        try:
            with self.assertRaises(ValueError):
                self.wiki.checked_path(self.root, 'docs/linked/README.md')
        finally:
            self.assertTrue(junction.absolute().is_relative_to(self.root.absolute()))
            os.rmdir(junction)


if __name__ == '__main__':
    unittest.main()
