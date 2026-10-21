#!/usr/bin/env python3
"""Upload immutable update assets separately; never use GitHub's clobber option."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import urllib.parse

def digest_file(path):
    digest=hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda:source.read(65536),b''):digest.update(chunk)
    return digest.hexdigest()

def upload_immutable(paths, existing, remote_digest, upload):
    remote={}
    for asset in existing:
        if asset['name'] in remote:raise ValueError('Ambiguous existing asset name')
        remote[asset['name']]=asset
    pending=[]
    unchanged=[]
    # Preflight the entire group before uploading anything.
    for path in paths:
        if path.is_symlink() or not path.is_file():raise ValueError('Invalid immutable artifact')
        prior=remote.get(path.name)
        if prior is None:
            pending.append(path)
            continue
        if prior['size'] != path.stat().st_size:raise ValueError('Immutable asset already exists with different bytes: '+path.name)
        published=prior.get('digest') or ''
        digest=published[7:].lower() if re.fullmatch(r'sha256:[a-fA-F0-9]{64}',published) else remote_digest(prior)
        if digest != digest_file(path):raise ValueError('Immutable asset already exists with different bytes: '+path.name)
        unchanged.append(path.name)
    for path in pending:upload(path)
    return {'uploaded':[p.name for p in pending],'unchanged':unchanged}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets',type=Path,required=True)
    parser.add_argument('--repository',required=True)
    parser.add_argument('--tag',required=True)
    args=parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+',args.repository):parser.error('Invalid repository')
    endpoint='repos/'+args.repository+'/releases/tags/'+urllib.parse.quote(args.tag,safe='')
    existing=json.loads(subprocess.run(['gh','api',endpoint],capture_output=True,text=True,check=True).stdout)['assets']
    def remote_digest(asset):
        asset_id=asset['id']
        if type(asset_id) is not int or asset_id<=0:raise ValueError('Invalid GitHub asset ID')
        with tempfile.TemporaryFile() as output:
            subprocess.run(['gh','api','-H','Accept: application/octet-stream','repos/'+args.repository+'/releases/assets/'+str(asset_id)],stdout=output,check=True)
            output.seek(0)
            digest=hashlib.sha256()
            for chunk in iter(lambda:output.read(65536),b''):digest.update(chunk)
            return digest.hexdigest()
    def upload(path):
        # Existing-name races fail safely at GitHub; there is deliberately no --clobber.
        subprocess.run(['gh','release','upload',args.tag,str(path),'--repo',args.repository],check=True)
    print(json.dumps(upload_immutable(sorted(args.assets.iterdir()),existing,remote_digest,upload)))

if __name__=='__main__':main()
