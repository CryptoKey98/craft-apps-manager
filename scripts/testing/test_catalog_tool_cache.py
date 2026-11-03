"""Executable cache reuse must preserve exact source identity and file integrity."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('cache', Path(__file__).resolve().parents[1]/'publishing/catalog_tool_cache.py')
cache = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cache)


class CacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='craft publisher cache ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root/'built.exe'
        self.source.write_bytes(b'MZ verified fixture executable')
        self.directory = self.root/'cache'
        self.output = self.root/'release'/'app-release-catalog-tool.exe'
        self.identity = 'windows-x64-source-one-compiler-one'

    def test_cold_cache_does_not_publish_a_binary(self):
        self.assertFalse(cache.restore(self.directory, self.output, self.identity))
        self.assertFalse(self.output.exists())

    def test_exact_revision_and_checksum_restore_the_built_binary(self):
        cache.store(self.directory, self.source, self.identity)
        self.assertTrue(cache.restore(self.directory, self.output, self.identity))
        self.assertEqual(self.output.read_bytes(), self.source.read_bytes())
        self.assertEqual({p.name for p in self.directory.iterdir()}, {'tool.exe','receipt.json'})

    def test_changed_source_or_compiler_cannot_reuse_previous_executable(self):
        cache.store(self.directory, self.source, self.identity)
        for identity in ['windows-x64-source-two-compiler-one', 'windows-x64-source-one-compiler-two']:
            self.assertFalse(cache.restore(self.directory, self.output, identity))
            self.assertFalse(self.output.exists())

    def test_modified_cached_binary_is_rejected_before_publication(self):
        cache.store(self.directory, self.source, self.identity)
        (self.directory/'tool.exe').write_bytes(b'MZ changed fixture executable!')
        self.assertFalse(cache.restore(self.directory, self.output, self.identity))
        self.assertFalse(self.output.exists())

    def test_incomplete_and_malformed_cache_is_a_miss(self):
        cache.store(self.directory, self.source, self.identity)
        (self.directory/'receipt.json').write_text('{bad json')
        self.assertFalse(cache.restore(self.directory, self.output, self.identity))
        cache.store(self.directory, self.source, self.identity)
        (self.directory/'tool.exe').unlink()
        self.assertFalse(cache.restore(self.directory, self.output, self.identity))

    def test_rebuild_replaces_damaged_cache_and_removes_stale_receipt(self):
        cache.store(self.directory, self.source, self.identity)
        self.source.write_bytes(b'MZ newly built fixture executable')
        cache.store(self.directory, self.source, self.identity)
        self.assertTrue(cache.restore(self.directory, self.output, self.identity))
        self.assertEqual(self.output.read_bytes(), self.source.read_bytes())
        receipt=json.loads((self.directory/'receipt.json').read_text())
        self.assertEqual(receipt['sha256'],hashlib.sha256(self.source.read_bytes()).hexdigest())

    def test_workflow_cli_reports_cold_and_warm_decisions(self):
        output = self.root/'github-output'
        command=[sys.executable,str(Path(SPEC.origin)), '--directory', str(self.directory),
                 '--binary',str(self.source),'--identity',self.identity]
        env=dict(os.environ,GITHUB_OUTPUT=str(output))
        subprocess.run([*command,'restore'],env=env,capture_output=True,text=True,check=True)
        self.assertEqual(output.read_text(),'reused=false\n')
        subprocess.run([*command,'store'],env=env,capture_output=True,text=True,check=True)
        subprocess.run([*command,'restore'],env=env,capture_output=True,text=True,check=True)
        self.assertEqual(output.read_text(),'reused=false\nreused=true\n')

    def test_invalid_receipts_do_not_replace_an_existing_output(self):
        cache.store(self.directory, self.source, self.identity)
        self.output.parent.mkdir()
        self.output.write_bytes(b'previous output must survive a rejected cache')
        original=json.loads((self.directory/'receipt.json').read_text())
        invalid=[None, [], {**original,'schema':True}, {**original,'size':'12'},
                 {**original,'sha256':'00'*32}]
        for receipt in invalid:
            with self.subTest(receipt=receipt):
                (self.directory/'receipt.json').write_text(json.dumps(receipt))
                self.assertFalse(cache.restore(self.directory, self.output, self.identity))
                self.assertEqual(self.output.read_bytes(), b'previous output must survive a rejected cache')


if __name__ == '__main__':
    unittest.main()
