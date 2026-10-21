"""Local publisher tests: no uploads, credentials, or real installations."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import copy
import importlib.util
import json
from pathlib import Path
import unittest
import tempfile
from unittest import mock

ROOT = project_root()
spec = importlib.util.spec_from_file_location("update_catalog", ROOT / "scripts/publishing/update-catalog.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class CatalogTests(unittest.TestCase):
    def setUp(self):
        self.catalog = json.loads((ROOT / "tests/fixtures/updater-v2/catalog.json").read_text())

    def test_release_preflight_rejects_stale_standby_policy(self):
        with tempfile.TemporaryDirectory(dir=ROOT / 'target') as folder:
            folder=Path(folder)
            catalog=folder/'catalog.json'; profiles=folder/'profiles.json'
            catalog.write_text(json.dumps(self.catalog)); profiles.write_text(json.dumps([{'repository':'owner/manager','channel':'stable'}]))
            effective={'source':'service','publishing':{'enabled':True},'repository':'owner/manager','channel':'stable','next_catalog_path':str(catalog),'profiles_path':str(profiles)}
            with mock.patch.dict(module.os.environ,{'CRAFT_RELEASE_TAG':'v1.0.0'}), mock.patch.object(module,'validate_routes'):
                with self.assertRaisesRegex(ValueError,'release tag'):
                    module.publication_preflight(effective,None)

    def test_renewal_increases_sequence(self):
        renewed = module.renew(self.catalog, 2, 2000)
        self.assertEqual(renewed["sequence"], 2)
        self.assertEqual(renewed["expires"], 2000 + 30 * 86400)
        self.assertEqual(renewed["packages"], self.catalog["packages"])
        with self.assertRaises(ValueError): module.renew(self.catalog, 1, 2000)

    def test_required_bridge_cannot_be_pruned(self):
        changed = copy.deepcopy(self.catalog)
        changed["packages"] = changed["packages"][1:]
        with self.assertRaises(ValueError): module.retain_bridges(changed, self.catalog)

    def test_partial_platform_revision_preserves_others(self):
        changed = copy.deepcopy(self.catalog)
        changed["packages"][0]["revision"] = 2
        result = module.renew(changed, 2, 2000)
        self.assertEqual(result["packages"][1], self.catalog["packages"][1])

    def test_duplicate_ids_rejected(self):
        self.catalog["packages"].append(self.catalog["packages"][0])
        with self.assertRaises(ValueError): module.validate_shape(self.catalog)

if __name__ == "__main__": unittest.main()
