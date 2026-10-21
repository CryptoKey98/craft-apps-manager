"""Regression fixtures for independent updater configuration review. No installs."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import copy
import importlib.util
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = project_root()

def load(name, file):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

adapter = load("review_config", "lib/updater_config.py")
publisher = load("review_publisher", "publishing/update-catalog.py")

def config():
    return {"source":"service", "repository":"owner/manager", "channel":"stable", "revision":2,
            "public_key_fingerprint":"a"*64, "runtime_config_fingerprint":"b"*64}

def identity(c=None, target="x86_64-apple-darwin"):
    c = c or config()
    return {"version":"1.2.3", "revision":c["revision"], "target":target,
            "update_source":c["source"], "update_repository":c["repository"],
            "update_channel":c["channel"], "update_key_fingerprint":c["public_key_fingerprint"],
            "update_config_fingerprint":c["runtime_config_fingerprint"]}

def thin(value, cpu):
    data=json.dumps(value).encode()
    offset=32+72+80
    header=struct.pack("<IiiIIIII",0xfeedfacf,cpu,3,2,1,152,0,0)
    segment=struct.pack("<II16sQQQQiiII",0x19,152,b"__TEXT",0,offset+len(data),0,offset+len(data),7,5,1,0)
    section=struct.pack("<16s16sQQIIIIIIII",b"__craft_update",b"__TEXT",0,len(data),offset,0,0,0,0,0,0,0)
    return header+segment+section+data

def fat(first,second):
    offset=48
    return (struct.pack(">II",0xcafebabe,2)+struct.pack(">iiIII",0x01000007,3,offset,len(first),0)
            +struct.pack(">iiIII",0x0100000c,0,offset+len(first),len(second),0)+first+second)

class ReviewTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        (ROOT/"target/updater-config-checks").mkdir(parents=True,exist_ok=True)

    def test_stale_runtime_configuration_is_rejected(self):
        c=config(); old=identity(c); c["runtime_config_fingerprint"]="c"*64
        with self.assertRaises(ValueError): adapter.verify_identity(old,c,"1.2.3")

    def test_both_macos_slices_are_verified_without_execution(self):
        c=config(); arm=identity(c,"aarch64-apple-darwin"); intel=identity(c)
        good=fat(thin(intel,0x01000007),thin(arm,0x0100000c))
        adapter.verify_macos_bytes(good,c,"1.2.3",True)
        for field in ("update_source","update_key_fingerprint","update_config_fingerprint","revision"):
            bad=copy.deepcopy(arm);bad[field]="wrong"
            with self.assertRaises(ValueError):
                adapter.verify_macos_bytes(fat(thin(intel,0x01000007),thin(bad,0x0100000c)),c,"1.2.3",True)
        with self.assertRaises(ValueError): adapter.verify_macos_bytes(thin(intel,0x01000007),c,"1.2.3",True)
        with self.assertRaises(ValueError): adapter.verify_macos_bytes(good[:-1],c,"1.2.3",True)

    def test_selected_formats_gate_msi_smoke_and_arch_job(self):
        text=(ROOT/".github/workflows/release.yml").read_text(encoding="utf-8")
        smoke=text.split("- name: Verify upgrade from the previous MSI release",1)[1].split("shell:",1)[0]
        self.assertIn("needs.validate.outputs.windows_msi == 'true'",smoke)
        arch=text.split("  arch:\n",1)[1].split("    needs:",1)[0]
        self.assertIn("needs.validate.outputs.linux_arch == 'true'",arch)

    def test_macos_packager_requires_all_slices_before_signing(self):
        text=(ROOT/"scripts/packaging/package-macos.sh").read_text()
        self.assertLess(text.index("--require-universal"),text.index("codesign --force"))
        self.assertIn("--require-universal",(ROOT/"scripts/packaging/package-macos-dmg.sh").read_text())

    def test_activated_release_preflights_publication_before_package_jobs(self):
        text=(ROOT/".github/workflows/release.yml").read_text(encoding="utf-8")
        validate=text.split("  validate:",1)[1].split("  windows:",1)[0]
        self.assertIn("scripts/publishing/update-catalog.py preflight",validate)
        self.assertIn("vars.CRAFT_UPDATE_V2_ENABLED == 'true'",validate)

    def test_publisher_uses_the_preflighted_release_source(self):
        release=(ROOT/".github/workflows/release.yml").read_text(encoding="utf-8")
        call=release.split("  update-catalog:",1)[1]
        self.assertIn("source_ref: ${{ inputs.source_ref || github.sha }}",call)
        renew=(ROOT/".github/workflows/renew-update-catalog.yml").read_text(encoding="utf-8")
        self.assertEqual(renew.count("ref: ${{ inputs.source_ref || github.ref }}"),2)

    def test_publication_readiness_rejects_disabled_or_wrong_scope(self):
        c=config();c.update(publishing={"enabled":False},public_key_hex="07"*32)
        with self.assertRaises(ValueError): publisher.publication_preflight(c,None)
        with tempfile.TemporaryDirectory(dir=ROOT/"target/updater-config-checks") as folder:
            folder=Path(folder); catalog=folder/"catalog.json"; profiles=folder/"profiles.json"
            catalog.write_text(json.dumps({"schema":2,"sequence":1,"repository":"wrong/repo","channel":"stable","releases":[],"packages":[],"routes":[]}))
            profiles.write_text(json.dumps([{"repository":"owner/manager","channel":"stable"}]))
            c.update(publishing={"enabled":True},next_catalog_path=str(catalog),profiles_path=str(profiles))
            with self.assertRaises(ValueError): publisher.publication_preflight(c,None)
            with patch.object(publisher,"validate_routes") as validate:
                value=json.loads(catalog.read_text());value["repository"]="owner/manager";catalog.write_text(json.dumps(value))
                publisher.publication_preflight(c,None)
                validate.assert_called_once()
            profiles.write_text('[{"repository":"wrong/repo","channel":"stable"}]')
            with self.assertRaises(ValueError): publisher.publication_preflight(c,None)

    def test_arch_passes_resolved_revision_to_makepkg(self):
        shell=shutil.which("bash")
        git_bash=Path("C:/Program Files/Git/bin/bash.exe")
        if os.name=="nt": shell=str(git_bash) if git_bash.exists() else None
        if not shell: self.skipTest("Bash unavailable")
        with tempfile.TemporaryDirectory(dir=ROOT/"target/updater-config-checks") as folder:
            root=Path(folder); (root/"scripts/packaging").mkdir(parents=True); (root/"tools").mkdir()
            script=root/"scripts/packaging/package-linux-arch.sh";shutil.copyfile(ROOT/"scripts/packaging/package-linux-arch.sh",script)
            (root/"Cargo.toml").write_text('version = "1.2.3"\n')
            for rel in ["assets/icon.png","assets/fonts/OFL.txt","assets/app-icons/license.txt","docs/user-guide/linux.md","LICENSE","THIRD-PARTY-NOTICES.txt","packaging/linux/craft-apps-manager.desktop","packaging/linux/craft-apps-builder.desktop"]:
                p=root/rel;p.parent.mkdir(parents=True,exist_ok=True);p.write_text("fixture")
            shutil.copyfile(ROOT/"packaging/linux/PKGBUILD",root/"packaging/linux/PKGBUILD")
            elf=bytearray(64);elf[:6]=bytes.fromhex("7f454c460201");elf[18:20]=(62).to_bytes(2,"little");(root/"manager").write_bytes(elf)
            (root/"tools/python3").write_bytes(b'#!/bin/sh\ncase "$*" in *" format "*) echo true;; *" revision "*) echo 7;; *" verify "*) exit 0;; *) exit 99;; esac\n')
            (root/"tools/makepkg").write_bytes(b'#!/bin/bash\nsource PKGBUILD\nprintf "%s" "$pkgrel" > "craft-apps-manager-$pkgver-$pkgrel-x86_64.pkg.tar.zst"\n')
            for p in (root/"tools").iterdir(): p.chmod(0o755)
            env=dict(os.environ);env.pop("CRAFT_BUILD_REVISION",None)
            env["PATH"]=str(root/"tools")+os.pathsep+env["PATH"]
            run=subprocess.run([shell,str(script),str(root/"manager"),str(root/"out")],env=env,capture_output=True,text=True)
            self.assertEqual(run.returncode,0,run.stdout+run.stderr)
            self.assertEqual((root/"out/craft-apps-manager-1.2.3-7-x86_64.pkg.tar.zst").read_text(),"7")

if __name__=="__main__":unittest.main()
