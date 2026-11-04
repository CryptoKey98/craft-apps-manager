"""Installer-side directory retention must also protect already-shipped updaters."""
from pathlib import Path
import os
import shutil
import subprocess
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]

class LocationPackaging(unittest.TestCase):
    @unittest.skipUnless(shutil.which('pwsh'), 'PowerShell is required')
    def test_upgrade_smoke_uses_configured_revision_without_environment_override(self):
        smoke = (ROOT / 'scripts/testing/smoke-windows-msi.ps1').read_text(encoding='utf-8-sig')
        assignment = next(line.strip() for line in smoke.splitlines()
                          if line.strip().startswith('$expectedRevision ='))
        env = dict(os.environ)
        env.pop('CRAFT_BUILD_REVISION', None)
        expected = subprocess.check_output(
            ['python', 'scripts/lib/updater_config.py', 'revision', '--view', 'packaging',
             '--target-os', 'windows'], cwd=ROOT, env=env, text=True).strip()
        actual = subprocess.check_output(
            ['pwsh', '-NoProfile', '-Command',
             "$ErrorActionPreference = 'Stop'; $projectRoot = (Get-Location).Path; "
             + assignment + '; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; Write-Output $expectedRevision'],
            cwd=ROOT, env=env, text=True).strip()
        self.assertEqual(actual, expected,
                         'The MSI smoke test must use the same revision resolver as packaging')

    def test_registered_folder_is_restored_for_uninstall_as_well_as_upgrade(self):
        package = (ROOT / 'scripts/packaging/package-windows-msi.ps1').read_text(encoding='utf-8-sig')
        xml = package.split('$xml = @"', 1)[1].split('"@', 1)[0]
        document = ET.fromstring(xml)
        namespace = {'w': 'http://wixtoolset.org/schemas/v4/wxs'}
        for sequence in ['InstallUISequence', 'InstallExecuteSequence']:
            action = document.find(f'w:Package/w:{sequence}/w:Custom[@Action="PreserveManagerInstallLocation"]', namespace)
            self.assertIsNotNone(action, sequence)
            self.assertEqual(action.attrib.get('Before'), 'CostFinalize')
            self.assertNotIn('Condition', action.attrib,
                             'Uninstall must resolve the registered folder before costing file removal')

    def test_incoming_msi_preserves_location_before_old_product_removal(self):
        package = (ROOT / 'scripts/packaging/package-windows-msi.ps1').read_text(encoding='utf-8-sig')
        self.assertIn('msi-install-location.rs', package)
        self.assertIn('BinaryRef="ManagerInstallLocation"', package)
        self.assertIn('DllEntry="PreserveManagerInstallLocation"', package)
        self.assertIn('Before="CostFinalize"', package)
        self.assertIn('Return="check"', package)

    def test_upgrade_smoke_covers_custom_folder_and_data_preservation(self):
        smoke = (ROOT / 'scripts/testing/smoke-windows-msi.ps1').read_text(encoding='utf-8-sig')
        self.assertIn('custom-install', smoke)
        self.assertIn('preservation-marker.txt', smoke)
        self.assertIn('INSTALLFOLDER=', smoke)
        self.assertIn('test_msi_location.py', smoke)

if __name__ == '__main__':
    unittest.main()
