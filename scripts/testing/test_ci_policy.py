"""CI routing and required-job failure boundaries, without GitHub writes."""
import importlib.util
import json
import os
import re
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('ci_policy', ROOT / 'scripts/testing/ci_policy.py')
policy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(policy)


class PolicyTests(unittest.TestCase):
    def test_documentation_skips_compilers_and_packages(self):
        result = policy.classify(['README.md', 'docs/development/building.md', 'docs/images/main.png'])
        self.assertEqual(result, {'core': False, 'packages': False, 'metadata': False, 'catalog': False})

    def test_changelog_runs_metadata_without_application_builds(self):
        result = policy.classify(['CHANGELOG.md'])
        self.assertTrue(result['metadata'])
        self.assertFalse(result['core'])
        self.assertFalse(result['packages'])

    def test_ui_changes_run_core_but_not_native_transactions(self):
        result = policy.classify(['src/ui/screens/app_page.rs', 'README.md'])
        self.assertTrue(result['core'])
        self.assertFalse(result['packages'])

    def test_critical_and_unknown_changes_request_full_checks(self):
        for path in ['src/self_update/mod.rs', 'src/installations/packages/native/mod.rs',
                     'src/storage/settings.rs', 'src/operations/jobs.rs', 'src/app/cli.rs', 'src/network/mod.rs',
                     'scripts/packaging/package-linux-deb.sh', 'packaging/linux/PKGBUILD',
                     'Cargo.toml', 'Cargo.lock', 'build.rs', 'updater.toml', 'updater.production.toml',
                     'crates/craft-updater-service/src/lib.rs', '.github/workflows/check.yml',
                     'scripts/testing/ci_policy.py', 'new-unrecognized-file']:
            with self.subTest(path=path):
                result = policy.classify([path])
                self.assertTrue(result['core'])
                self.assertTrue(result['packages'])

    def test_modes_are_explicit(self):
        self.assertEqual(policy.classify([], mode='full'),
                         {'core': True, 'packages': True, 'metadata': True, 'catalog': False})
        self.assertEqual(policy.classify([], mode='catalog'),
                         {'core': False, 'packages': False, 'metadata': False, 'catalog': True})
        with self.assertRaises(ValueError):
            policy.classify([], mode='unsupported')

    def needs(self, flags):
        names = policy.ALL_JOBS
        needed = policy.required_jobs(flags)
        result = {name: {'result': 'success' if name in needed else 'skipped'} for name in names}
        result['changes'] = {'result': 'success', 'outputs': {k: str(v).lower() for k, v in flags.items()}}
        return result

    def test_gate_passes_requested_jobs_only(self):
        for mode, paths in [('changed', ['README.md']), ('changed', ['src/ui/mod.rs']),
                            ('full', []), ('catalog', [])]:
            with self.subTest(mode=mode):
                policy.verify_gate(self.needs(policy.classify(paths, mode)))

    def test_gate_rejects_skipped_failed_cancelled_or_absent_required_job(self):
        for paths in [['README.md'], ['src/ui/mod.rs'], ['Cargo.toml']]:
            for status in ['skipped', 'failure', 'cancelled', 'missing']:
                flags = policy.classify(paths)
                for name in policy.required_jobs(flags):
                    with self.subTest(paths=paths, status=status, job=name):
                        needs = self.needs(flags)
                        if status == 'missing':
                            needs.pop(name)
                        else:
                            needs[name]['result'] = status
                        with self.assertRaises(ValueError):
                            policy.verify_gate(needs)

    def test_gate_rejects_bad_detection_outputs_and_unexpected_failures(self):
        for output in [{}, {'core': 'banana'}, {'core': 'false', 'packages': 'true', 'metadata': 'false', 'catalog': 'false'}]:
            needs = self.needs(policy.classify(['README.md']))
            needs['changes']['outputs'] = output
            with self.assertRaises(ValueError):
                policy.verify_gate(needs)
        needs = self.needs(policy.classify(['README.md']))
        needs['windows']['result'] = 'failure'
        with self.assertRaises(ValueError):
            policy.verify_gate(needs)

    def test_git_diff_includes_deleted_files_and_both_sides_of_renames(self):
        folder = ROOT / 'target/ci-reorganization'
        folder.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=folder) as temporary:
            root = Path(temporary)
            env = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT='0')
            def git(*args):
                return subprocess.check_output(['git', '-c', 'init.templateDir=', '-c', 'core.autocrlf=false', *args], cwd=root, env=env, text=True).strip()
            git('init', '--quiet')
            git('config', 'user.name', 'CI Policy Fixture')
            git('config', 'user.email', 'ci-policy@example.invalid')
            (root / 'build.rs').write_text('critical fixture')
            git('add', '--', 'build.rs')
            git('commit', '--quiet', '-m', 'Fixture base')
            base = git('rev-parse', 'HEAD')
            (root / 'build.rs').rename(root / 'README.md')
            git('add', '--all')
            git('commit', '--quiet', '-m', 'Fixture rename')
            head = git('rev-parse', 'HEAD')
            paths = policy.changed_paths(root, base, head)
            self.assertEqual(set(paths), {'build.rs', 'README.md'})
            self.assertTrue(policy.classify(paths)['packages'])
            with self.assertRaises(ValueError):
                policy.changed_paths(root, '--malicious-option', head)

    def test_event_inputs_are_not_silently_treated_as_docs_only(self):
        self.assertEqual(policy.event_base('pull_request', {'pull_request': {'base': {'sha': 'a' * 40}}}), 'a' * 40)
        self.assertIsNone(policy.event_base('push', {'before': '0' * 40}))
        self.assertIsNone(policy.event_base('workflow_dispatch', {}))
        for event, payload in [('pull_request', {}), ('push', {}), ('unknown', {})]:
            with self.assertRaises(ValueError):
                policy.event_base(event, payload)

    def test_workflow_routes_every_job_and_aggregates_all_results(self):
        text = (ROOT / '.github/workflows/check.yml').read_text(encoding='utf-8')
        matches = list(re.finditer(r'^  ([\w-]+):\n', text, re.M))
        jobs = {match[1]: text[match.end():matches[index + 1].start() if index + 1 < len(matches) else len(text)]
                for index, match in enumerate(matches) if match.start() > text.index('\njobs:')}
        self.assertEqual(set(jobs), policy.ALL_JOBS | {'gate'})
        needs = re.search(r'    needs: \[([^\]]+)\]', jobs['gate'])[1]
        self.assertEqual({name.strip() for name in needs.split(',')}, policy.ALL_JOBS)
        self.assertIn('if: always()', jobs['gate'])
        self.assertIn('CI_NEEDS: ${{ toJSON(needs) }}', jobs['gate'])
        for job in policy.CORE_JOBS:
            self.assertIn("if: needs.changes.outputs.core == 'true'", jobs[job])
        for job in policy.PACKAGE_JOBS:
            self.assertIn("if: needs.changes.outputs.packages == 'true'", jobs[job])
        self.assertIn("if: needs.changes.outputs.metadata == 'true'", jobs['release-metadata'])
        self.assertIn("cancel-in-progress: ${{ github.event_name == 'pull_request' }}", text)
        self.assertNotIn('paths-ignore:', text)


if __name__ == '__main__':
    unittest.main()
