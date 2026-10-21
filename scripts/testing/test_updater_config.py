"""Local config/publisher/packaging contract tests; no uploads or installers."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import importlib.util, json, unittest
from pathlib import Path
from unittest.mock import patch
ROOT=project_root()
spec=importlib.util.spec_from_file_location("updater_config",ROOT/"scripts/lib/updater_config.py")
module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
class ConfigTests(unittest.TestCase):
 def config(self): return {"schema_version":1,"source":"github","repository":"owner/repo","channel":"stable","revision":2,"public_key_fingerprint":"ab","runtime_config_fingerprint":"cd","required_features":[]}
 def test_source_and_feature_arguments_are_separate(self):
  c=self.config(); self.assertNotIn("--features",module.build_command(c,"x86_64-pc-windows-msvc")); c.update(source="service",required_features=["service-updater"]); args=module.build_command(c,"x86_64-pc-windows-msvc"); self.assertEqual(args[args.index("--features")+1],"service-updater")
 def test_identity_requires_same_source_key_scope_revision(self):
  c=self.config(); identity={"version":"1.2.3","revision":2,"update_source":"github","update_repository":"owner/repo","update_channel":"stable","update_key_fingerprint":"ab","update_config_fingerprint":"cd"}; module.verify_identity(identity,c,"1.2.3")
  for field in ("revision","update_source","update_repository","update_channel","update_key_fingerprint","update_config_fingerprint"):
   bad=dict(identity);bad[field]="wrong"
   with self.assertRaises(ValueError):module.verify_identity(bad,c,"1.2.3")
 def test_renewal_due_boundaries(self):
  self.assertFalse(module.renewal_due(1000000,1,7));self.assertTrue(module.renewal_due(1000000,1000000-7*86400,7));self.assertTrue(module.renewal_due(10,20,7))
 def test_export_calls_standalone_tool_without_shell(self):
  c=self.config()
  with patch.object(module.subprocess,"check_output",return_value=json.dumps(c)) as command:
   self.assertEqual(module.load_view("build","linux"),c)
   argv=command.call_args.args[0];self.assertIn("craft-updater-config",argv);self.assertIn("--project-root",argv);self.assertNotIn("shell",command.call_args.kwargs)
 def test_unknown_runtime_feature_is_not_an_executable_instruction(self):
  c=self.config();c.update(source="service",required_features=["run-arbitrary-command"])
  with self.assertRaises(ValueError):module.build_command(c,None)
 def test_release_and_packagers_share_resolver(self):
  release=(ROOT/".github/workflows/release.yml").read_text()
  self.assertIn("scripts/lib/updater_config.py build",release)
  self.assertNotIn("--config update-service/catalog-config.json",release)
  for path in [ROOT/"scripts/packaging/package-macos.sh",ROOT/"scripts/packaging/package-linux-zip.sh"]:
   self.assertIn("updater_config.py",path.read_text())
 def test_production_worker_has_one_generated_config(self):
  self.assertFalse((ROOT/"update-service/wrangler.jsonc").exists())
  self.assertTrue((ROOT/"update-service/wrangler.test.jsonc").exists())
 def test_selected_file_is_forwarded_to_actual_cargo_build(self):
  with patch.dict(module.os.environ,{},clear=True):
   self.assertEqual(module.build_environment("custom.toml")["CRAFT_UPDATER_CONFIG"],"custom.toml")
 def test_empty_actions_overrides_do_not_clear_file_configuration(self):
  with patch.dict(module.os.environ,{"CRAFT_RELEASE_CATALOG_URL":""},clear=True):
   self.assertNotIn("CRAFT_UPDATE_CATALOG_URL",module.build_environment())
  with patch.dict(module.os.environ,{"CRAFT_RELEASE_CATALOG_URL":"https://new.example/catalog"},clear=True):
   self.assertEqual(module.build_environment()["CRAFT_UPDATE_CATALOG_URL"],"https://new.example/catalog")
if __name__=="__main__":unittest.main()
