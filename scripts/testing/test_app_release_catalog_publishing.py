"""Exercise publication ordering, signature verification, and owned-file retention."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import time
import unittest
from unittest import mock
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('app_release_catalog', ROOT/'scripts/publishing/app_release_catalog.py')
catalog = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(catalog)


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='craft app catalog publishing ')
        self.root = Path(self.temporary.name)
        self.feed = self.root/'feed'
        self.feed.mkdir()
        self.key = Ed25519PrivateKey.from_private_bytes(bytes([81])*32)
        public = self.key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()
        self.source = self.root/'source.json'
        self.source.write_text(json.dumps(dict(schema=1, enabled=False, index_url='https://raw.githubusercontent.com/fixture/manager/app-release-catalog/v1/index.json', keys={'fixture':public})))
        self.tool = Path(os.environ.get('APP_CATALOG_TOOL', ROOT/'target/release'/('app-release-catalog-tool.exe' if os.name=='nt' else 'app-release-catalog-tool')))
        if not self.tool.is_file():
            self.fail('Build app-release-catalog-tool before running publication tests')
        self.now = int(time.time())

    def tearDown(self):
        self.temporary.cleanup()

    def publication(self, sequence, version, at=None):
        at = self.now if at is None else at
        output = self.root/f'published-{sequence}-{version}'
        output.mkdir()
        (output/'apps').mkdir()
        app = dict(schema=1, repository='fixture/app', latest_tag=version, releases=[dict(tag_name=version,name=None,draft=False,prerelease=False,assets=[])], scopes=[])
        body = json.dumps(app,separators=(',',':')).encode()
        digest = hashlib.sha256(body).hexdigest()
        (output/'apps'/f'{digest}.json').write_bytes(body)
        index = dict(schema=1,purpose='app-release-catalog',sequence=sequence,generated_at=at,repositories={'fixture/app':dict(sha256=digest,size=len(body),checked_at=at)})
        payload = json.dumps(index,separators=(',',':'))
        signature = self.key.sign(b'craft-apps-manager/app-release-catalog/v1\n'+payload.encode()).hex()
        (output/'index.json').write_text(json.dumps(dict(key_id='fixture',payload=payload,signature=signature)))
        candidate = self.root/f'candidate-{sequence}-{version}'
        candidate.mkdir()
        (candidate/'bundle.json').write_text(json.dumps(dict(index=index,objects={digest:app})))
        (candidate/'repositories.json').write_text(json.dumps(['fixture/app']))
        (candidate/'discovery.json').write_text(json.dumps(dict(schema=1,repositories={},pages={})))
        (candidate/'inspection.json').write_text(json.dumps(dict(schema=1,observations=[],attempts={},errors={})))
        return output,candidate,digest

    def publish(self, sequence, version, at=None):
        output,candidate,digest = self.publication(sequence,version,at)
        catalog.stage_objects(self.feed,output,self.source,self.tool,now=at or self.now)
        catalog.stage_root(self.feed,output,candidate,self.source,self.tool,now=at or self.now)
        return output,candidate,digest

    def test_objects_are_available_before_the_root_changes(self):
        first,_,_ = self.publish(1,'v1.0.0')
        self.assertTrue((self.feed/'v1/index.json').is_file(), 'Verified publication must produce the catalog root')
        next_feed,candidate,digest = self.publication(2,'v2.0.0')
        old = (self.feed/'v1/index.json').read_bytes()
        # The process clock may tick while the fixture keeps its simulated time.
        with mock.patch.object(catalog.time, 'time', return_value=self.now + 1):
            catalog.stage_objects(self.feed,next_feed,self.source,self.tool,now=self.now)
        self.assertTrue((self.feed/'v1/apps'/f'{digest}.json').is_file())
        self.assertEqual((self.feed/'v1/index.json').read_bytes(),old)
        catalog.stage_root(self.feed,next_feed,candidate,self.source,self.tool,now=self.now)
        self.assertEqual((self.feed/'v1/index.json').read_bytes(),(next_feed/'index.json').read_bytes())
        self.assertEqual((self.feed/'v1/roots/1.json').read_bytes(),(first/'index.json').read_bytes())

    def test_invalid_signature_cannot_stage_objects_or_replace_root(self):
        self.publish(1,'v1.0.0')
        next_feed,_,digest = self.publication(2,'v2.0.0')
        envelope = json.loads((next_feed/'index.json').read_text())
        envelope['signature'] = '00'*64
        (next_feed/'index.json').write_text(json.dumps(envelope))
        before = (self.feed/'v1/index.json').read_bytes()
        with self.assertRaises(Exception):catalog.stage_objects(self.feed,next_feed,self.source,self.tool,now=self.now)
        self.assertFalse((self.feed/'v1/apps'/f'{digest}.json').exists())
        self.assertEqual((self.feed/'v1/index.json').read_bytes(),before)

    def test_immutable_object_conflicts_are_not_overwritten(self):
        self.publish(1,'v1.0.0')
        published,_,digest = self.publication(2,'v2.0.0')
        target = self.feed/'v1/apps'/f'{digest}.json'
        target.write_text('unrecognized bytes')
        with self.assertRaises(Exception):catalog.stage_objects(self.feed,published,self.source,self.tool,now=self.now)
        self.assertEqual(target.read_text(),'unrecognized bytes')

    def test_lower_or_conflicting_sequences_are_rejected(self):
        self.publish(5,'v5.0.0')
        before = (self.feed/'v1/index.json').read_bytes()
        for seq in [4,5]:
            published,candidate,_ = self.publication(seq,f'v{seq}.1.0')
            with self.assertRaises(Exception):catalog.stage_objects(self.feed,published,self.source,self.tool,now=self.now)
            with self.assertRaises(Exception):catalog.stage_root(self.feed,published,candidate,self.source,self.tool,now=self.now)
        self.assertEqual((self.feed/'v1/index.json').read_bytes(),before)

    def test_checkpoint_must_match_the_signed_index(self):
        self.publish(1,'v1.0.0')
        published,candidate,_ = self.publication(2,'v2.0.0')
        bundle=json.loads((candidate/'bundle.json').read_text());bundle['index']['sequence']=999
        (candidate/'bundle.json').write_text(json.dumps(bundle))
        catalog.stage_objects(self.feed,published,self.source,self.tool,now=self.now)
        before=(self.feed/'v1/index.json').read_bytes()
        with self.assertRaises(Exception):catalog.stage_root(self.feed,published,candidate,self.source,self.tool,now=self.now)
        self.assertEqual((self.feed/'v1/index.json').read_bytes(),before)

    def test_retention_preserves_recent_objects_and_unknown_files(self):
        _,_,old_digest=self.publish(1,'v1.0.0',self.now-31*86400)
        self.publish(2,'v2.0.0',self.now-31*86400+1)
        (self.feed/'v1/apps/user-note.txt').write_text('keep')
        unknown=hashlib.sha256(b'unknown').hexdigest()
        (self.feed/'v1/apps'/f'{unknown}.json').write_bytes(b'unknown')
        self.publish(3,'v3.0.0',self.now)
        self.assertFalse((self.feed/'v1/apps'/f'{old_digest}.json').exists())
        self.assertEqual((self.feed/'v1/apps/user-note.txt').read_text(),'keep')
        self.assertTrue((self.feed/'v1/apps'/f'{unknown}.json').exists())
        self.assertTrue((self.feed/'v1/roots/2.json').is_file(),'Previous root remains available regardless of age')

    def test_invalid_retention_paths_do_not_delete_unrelated_files(self):
        self.publish(1,'v1.0.0')
        ledger=self.feed/'publisher/retention.json'
        ledger.write_text(json.dumps(dict(schema=1,retired={'../../outside.txt':1})))
        outside=self.root/'outside.txt';outside.write_text('keep')
        published,candidate,_=self.publication(2,'v2.0.0')
        with self.assertRaises(Exception):catalog.stage_root(self.feed,published,candidate,self.source,self.tool,now=self.now)
        self.assertEqual(outside.read_text(),'keep')

    def test_future_retention_timestamp_cannot_replace_the_current_catalog(self):
        self.publish(1,'v1.0.0')
        published,candidate,digest=self.publication(2,'v2.0.0')
        catalog.stage_objects(self.feed,published,self.source,self.tool,now=self.now)
        ledger=self.feed/'publisher/retention.json'
        value=json.loads(ledger.read_text())
        value['retired'][digest]=self.now+1
        ledger.write_text(json.dumps(value))
        before=(self.feed/'v1/index.json').read_bytes()
        with self.assertRaisesRegex(ValueError,'Invalid catalog retention ledger'):
            catalog.stage_root(self.feed,published,candidate,self.source,self.tool,now=self.now)
        self.assertEqual((self.feed/'v1/index.json').read_bytes(),before)

    def test_interrupted_object_phase_is_owned_and_can_be_cleaned_later(self):
        self.publish(1,'v1.0.0',self.now-32*86400)
        abandoned,_,digest=self.publication(2,'v2.0.0',self.now-31*86400)
        catalog.stage_objects(self.feed,abandoned,self.source,self.tool,now=self.now-31*86400)
        self.assertTrue((self.feed/'v1/apps'/f'{digest}.json').is_file())
        self.publish(3,'v3.0.0',self.now)
        self.assertFalse((self.feed/'v1/apps'/f'{digest}.json').exists())

    def test_retention_cannot_use_an_unsigned_archived_root(self):
        self.publish(1,'v1.0.0')
        self.publish(2,'v2.0.0')
        archive=self.feed/'v1/roots/1.json'
        value=json.loads(archive.read_text());value['signature']='00'*64
        archive.write_text(json.dumps(value))
        published,candidate,_=self.publication(3,'v3.0.0')
        catalog.stage_objects(self.feed,published,self.source,self.tool,now=self.now)
        before=(self.feed/'v1/index.json').read_bytes()
        with self.assertRaises(Exception):catalog.stage_root(self.feed,published,candidate,self.source,self.tool,now=self.now)
        self.assertEqual((self.feed/'v1/index.json').read_bytes(),before,'All cleanup inputs must be verified before replacing authority')

    def test_public_source_configuration_rejects_wrong_origins_and_paths(self):
        repo,branch,_=catalog.configuration(self.source)
        self.assertEqual((repo,branch),('fixture/manager','app-release-catalog'))
        value=json.loads(self.source.read_text())
        for url in ['http://raw.githubusercontent.com/fixture/manager/app-release-catalog/v1/index.json',
                    'https://github.com/fixture/manager/app-release-catalog/v1/index.json',
                    'https://raw.githubusercontent.com/fixture/manager/main/v1/index.json',
                    'https://raw.githubusercontent.com/fixture/manager/app-release-catalog/v1/index.json?token=bad']:
            value['index_url']=url;self.source.write_text(json.dumps(value))
            with self.assertRaises(ValueError):catalog.configuration(self.source)

    def test_collection_cannot_receive_a_signing_secret(self):
        output=self.root/'candidate-with-secret'
        with mock.patch.dict(os.environ,{'APP_RELEASE_CATALOG_SIGNING_KEY':'fixture-never-use'}):
            with self.assertRaisesRegex(ValueError,'without a signing secret'):
                catalog.collect(self.tool,self.source,self.feed,output,8)
        self.assertFalse(output.exists())

    def test_bundled_baseline_uses_only_the_verified_publication(self):
        _,_,digest=self.publish(1,'v1.0.0')
        output=self.root/'baseline.json'
        catalog.baseline_file(self.feed,self.source,self.tool,output)
        result=json.loads(output.read_text())
        self.assertEqual(result['schema'],1)
        self.assertEqual(result['envelope'],(self.feed/'v1/index.json').read_text())
        self.assertEqual(hashlib.sha256(result['objects'][digest].encode()).hexdigest(),digest)
        unrelated=self.root/'keep.json';unrelated.write_text('{"keep":true}')
        with self.assertRaises(ValueError):catalog.baseline_file(self.feed,self.source,self.tool,unrelated)
        self.assertEqual(json.loads(unrelated.read_text()),{'keep':True})

    def test_unsigned_repository_checkpoint_does_not_control_enrollment(self):
        self.publish(1,'v1.0.0')
        (self.feed/'app-catalog/repositories.json').write_text(json.dumps(['unrelated/anything']))
        output=self.root/'collection';observed=[]
        original=catalog.invoke
        def inspect(tool,command,*args):
            if command=='verify':return original(tool,command,*args)
            if command=='enroll':
                observed.extend(json.loads(Path(args[1]).read_text()))
                raise RuntimeError('stop-after-enrollment-check')
            self.fail('No further work should run in this boundary fixture')
        with mock.patch.object(catalog,'invoke',side_effect=inspect):
            with self.assertRaisesRegex(RuntimeError,'stop-after-enrollment-check'):
                catalog.collect(self.tool,self.source,self.feed,output,8)
        self.assertIn('fixture/app',observed)
        self.assertNotIn('unrelated/anything',observed)
        self.assertFalse((output/'repositories-seed.json').exists())


if __name__=='__main__':unittest.main()
