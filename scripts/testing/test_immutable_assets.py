"""Immutable upload policy tests; no GitHub requests or uploads."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
import json
from unittest.mock import patch

spec=importlib.util.spec_from_file_location('immutable_assets',(project_root() / "scripts/publishing/immutable-assets.py"))
module=importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class ImmutableTests(unittest.TestCase):
    def test_tagless_draft_assets_are_resolved_by_release_id(self):
        results=[module.subprocess.CompletedProcess([],0,json.dumps({'databaseId':123,'tagName':'v1.0.0'})),module.subprocess.CompletedProcess([],0,json.dumps({'assets':[{'id':456,'name':'manager.zip','size':3}]}))]
        with patch.object(module.subprocess,'run',side_effect=results) as run:
            self.assertEqual(module.release_assets('owner/manager','v1.0.0'),[{'id':456,'name':'manager.zip','size':3}])
            self.assertIn('databaseId,tagName',run.call_args_list[0].args[0])
            self.assertIn('repos/owner/manager/releases/123',run.call_args_list[1].args[0])

    def test_revision_two_collision_cannot_replace_previous_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'manager-update.zip'
            path.write_bytes(b'revision two')
            old={'name':path.name,'size':len(b'revision one'),'digest':'sha256:'+hashlib.sha256(b'revision one').hexdigest()}
            uploads=[]
            with self.assertRaises(ValueError):module.upload_immutable([path],[old],lambda _:self.fail('digest supplied'),uploads.append)
            self.assertEqual(uploads,[])

    def test_identical_existing_asset_is_never_reuploaded(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'manager-update.zip';path.write_bytes(b'same')
            old={'name':path.name,'size':4,'digest':'sha256:'+hashlib.sha256(b'same').hexdigest()}
            uploads=[]
            result=module.upload_immutable([path],[old],lambda _:self.fail('digest supplied'),uploads.append)
            self.assertEqual(uploads,[])
            self.assertEqual(result['unchanged'],[path.name])

    def test_all_collisions_checked_before_any_upload(self):
        with tempfile.TemporaryDirectory() as directory:
            a=Path(directory)/'a.zip';a.write_bytes(b'new')
            b=Path(directory)/'b.zip';b.write_bytes(b'changed')
            uploads=[]
            with self.assertRaises(ValueError):module.upload_immutable([a,b],[{'name':b.name,'size':7,'digest':'sha256:'+'0'*64}],lambda _:self.fail('digest supplied'),uploads.append)
            self.assertEqual(uploads,[])

    def test_missing_asset_uploads_without_replacing_anything(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'new.zip';path.write_bytes(b'new')
            uploads=[];result=module.upload_immutable([path],[],lambda _:self.fail('absent asset'),uploads.append)
            self.assertEqual(uploads,[path]);self.assertEqual(result['uploaded'],[path.name])

if __name__=='__main__':unittest.main()
