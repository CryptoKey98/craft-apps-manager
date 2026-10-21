#!/usr/bin/env python3
"""Build and run isolated A/B/C archive/helper transactions; no real apps/native installers."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import argparse
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import time
import zipfile
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

ROOT = project_root()
MARKER = 'Craft updater isolated fixture'

def wait_exit(pid):
    if os.name == 'nt':
        import ctypes
        kernel=ctypes.WinDLL('kernel32',use_last_error=True)
        kernel.OpenProcess.restype=ctypes.c_void_p
        kernel.WaitForSingleObject.argtypes=[ctypes.c_void_p,ctypes.c_uint32]
        kernel.CloseHandle.argtypes=[ctypes.c_void_p]
        handle=kernel.OpenProcess(0x100000,False,int(pid))
        if not handle:
            if ctypes.get_last_error()==87:return
            raise OSError(ctypes.get_last_error(),'Cannot wait for fixture process')
        try:
            if kernel.WaitForSingleObject(handle,15000)!=0:raise RuntimeError('Fixture process did not exit')
        finally:kernel.CloseHandle(handle)
    else:
        deadline=time.monotonic()+15
        while time.monotonic()<deadline:
            try:os.kill(int(pid),0)
            except ProcessLookupError:return
            time.sleep(.05)
        raise RuntimeError('Fixture process did not exit')

def wait_version(data, version):
    deadline = time.monotonic() + 75
    while time.monotonic() < deadline:
        try:
            receipt=json.loads((data / 'fixture-receipt.json').read_text())
            if receipt['version'] == version:
                wait_exit(receipt['pid'])
                return
        except (FileNotFoundError, json.JSONDecodeError): pass
        time.sleep(.1)
    raise RuntimeError('Fixture did not restart at ' + version)

def run_case(base, binaries):
    install, data, artifacts = (base / name for name in ('install', 'data', 'artifacts'))
    for directory in (base, install, data, artifacts):
        directory.mkdir(exist_ok=True)
        (directory / '.craft-updater-fixture').write_text(MARKER)
    target = install / ('fixture-manager.exe' if os.name == 'nt' else 'fixture-manager')
    shutil.copy2(binaries['1.0.0'], target)
    preserved = {'settings.json': b'{"theme":"light","app":"keep","external":"untouched"}', 'library.marker': b'preserve library', 'portable.txt': b'portable settings marker'}
    for name, content in preserved.items(): (data / name).write_bytes(content)
    catalog = json.loads((ROOT / 'tests/fixtures/updater-v2/catalog.json').read_text())
    now = int(time.time())
    catalog.update(issued=now, expires=now + 86400)
    for package, version in zip(catalog['packages'], ['1.1.0', '1.2.0']):
        entry = package['layout']['root'] + '/' + package['layout']['entrypoint']
        artifact = artifacts / package['asset']
        payload = binaries[version].read_bytes()
        if package['handler'] == 'archive.zip.v1':
            with zipfile.ZipFile(artifact, 'x', zipfile.ZIP_DEFLATED) as archive:
                info = zipfile.ZipInfo(entry)
                info.external_attr = 0o100755 << 16
                archive.writestr(info, payload)
        else:
            with tarfile.open(artifact, 'x:gz', format=tarfile.USTAR_FORMAT) as archive:
                info = tarfile.TarInfo(entry)
                info.mode, info.size = 0o755, len(payload)
                archive.addfile(info, io.BytesIO(payload))
        package.update(size=artifact.stat().st_size, sha256=hashlib.sha256(artifact.read_bytes()).hexdigest())
    payload = json.dumps(catalog, separators=(',', ':'))
    key = Ed25519PrivateKey.from_private_bytes(bytes([7])*32)
    (base / 'catalog.json').write_text(json.dumps({'payload':payload,'signature':key.sign(payload.encode()).hex()}))
    (base / 'scenario.json').write_text(json.dumps({'catalog':'catalog.json','artifact_dir':'artifacts','approve':True}))
    for version in ('1.1.0', '1.2.0'):
        subprocess.run([str(target), '--root', str(data), '--scenario', str(base / 'scenario.json')], check=True, timeout=30)
        wait_version(data, version)
        for marker in install.glob('runtime/self-update/*/helper.pid'):wait_exit(int(marker.read_text()))
        for name, content in preserved.items():
            if (data / name).read_bytes() != content: raise RuntimeError('Data preservation failed: ' + name)
    return {'transaction':'A ZIP-only → B ZIP+tar.gz → C tar.gz', 'host':os.name, 'result':'passed', 'preserved':list(preserved), 'scope':'isolated fixture using production archive and legacy helper transaction; not a native-package or production-feed test'}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',type=Path,required=True)
    args=parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='craft-updater-fixture-') as folder:
        base=Path(folder)
        binaries={}
        for version in ('1.0.0','1.1.0','1.2.0'):
            env=dict(os.environ,CRAFT_UPDATER_FIXTURE_VERSION=version)
            feature='updater-test-fixture' if version=='1.0.0' else 'updater-test-tar'
            subprocess.run(['cargo','build','--locked','--features',feature,'--bin','update-fixture'],cwd=ROOT,env=env,check=True)
            source=ROOT/'target/debug'/('update-fixture.exe' if os.name=='nt' else 'update-fixture')
            copy=base/(version+source.suffix)
            shutil.copy2(source,copy)
            binaries[version]=copy
        evidence=run_case(base/'scenario',binaries)
        args.out.write_text(json.dumps(evidence,indent=2),encoding='utf-8')
        print('Isolated bridge transaction passed; settings and library markers preserved')

if __name__=='__main__':main()
