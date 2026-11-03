"""Installer-side directory retention must also protect already-shipped updaters."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]

class LocationPackaging(unittest.TestCase):
    def test_incoming_msi_preserves_location_before_old_product_removal(self):
        package = (ROOT / 'scripts/packaging/package-windows-msi.ps1').read_text(encoding='utf-8-sig')
        self.assertIn('msi-install-location.rs', package)
        self.assertIn('BinaryRef="ManagerInstallLocation"', package)
        self.assertIn('DllEntry="PreserveManagerInstallLocation"', package)
        self.assertIn('Before="CostFinalize"', package)
        self.assertIn('Return="check"', package)
        self.assertIn('Condition="NOT REMOVE', package)

    def test_upgrade_smoke_covers_custom_folder_and_data_preservation(self):
        smoke = (ROOT / 'scripts/testing/smoke-windows-msi.ps1').read_text(encoding='utf-8-sig')
        self.assertIn('custom-install', smoke)
        self.assertIn('preservation-marker.txt', smoke)
        self.assertIn('INSTALLFOLDER=', smoke)

if __name__ == '__main__':
    unittest.main()
