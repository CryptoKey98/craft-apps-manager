"""Prepare and retain signed app-release catalog files; never publish an app release."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[2]
RETENTION_SECONDS = 86400


def read_json(path, limit=128 * 1024 * 1024):
    path = Path(path)
    if path.is_symlink() or path.stat().st_size > limit:
        raise ValueError('Linked or oversized catalog input')
    def unique(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError('Duplicate catalog key')
            result[key] = value
        return result
    return json.loads(path.read_bytes(), object_pairs_hook=unique)


def safe(root, relative):
    root = Path(root)
    if root.is_symlink():
        raise ValueError('Linked output root')
    root = root.resolve()
    relative = Path(relative)
    if relative.is_absolute() or '..' in relative.parts:
        raise ValueError('Catalog path escaped its owned folder')
    path = root / relative
    for parent in [path, *path.parents]:
        if parent == root:
            break
        if parent.is_symlink():
            raise ValueError('Linked catalog output')
    return path


def write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + '.new')
    if temporary.exists() or temporary.is_symlink():
        raise ValueError('Unexpected publication temporary file')
    try:
        with temporary.open('xb') as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if temporary.is_file() and not temporary.is_symlink():
            temporary.unlink()


def invoke(tool, *args):
    result = subprocess.run([str(tool), *map(str, args)], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f'Catalog {args[0]} failed: {result.stderr[-2000:]}')
    return result.stdout.strip()


def configuration(source):
    value = read_json(source, 64 * 1024)
    url = urlparse(value.get('index_url') or '')
    parts = url.path.strip('/').split('/')
    if (url.scheme != 'https' or url.netloc != 'raw.githubusercontent.com' or
            url.query or url.fragment or len(parts) != 5 or
            parts[2:] != ['app-release-catalog', 'v1', 'index.json'] or
            not all(p not in ('.', '..') and re.fullmatch(r'[A-Za-z0-9_.-]+', p) for p in parts[:2]) or
            value.get('schema') != 1 or not isinstance(value.get('enabled'), bool) or
            not value.get('keys') or not all(re.fullmatch('[0-9a-f]{64}', key) for key in value['keys'].values())):
        raise ValueError('Invalid production app catalog configuration')
    return '/'.join(parts[:2]), parts[2], value


def verified(tool, source, index, objects):
    invoke(tool, 'verify', '--source', source, '--index', index, '--objects', objects)
    envelope = read_json(index, 256 * 1024)
    return json.loads(envelope['payload'])


def retained_index(path, keys):
    # Retained roots need signature checks, not a repeated read of every app
    # object for every historical heartbeat. Current data is fully verified by
    # the Rust publisher before either publication phase.
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    envelope = read_json(path, 256 * 1024)
    public = bytes.fromhex(keys[envelope['key_id']])
    payload = envelope['payload'].encode()
    Ed25519PublicKey.from_public_bytes(public).verify(bytes.fromhex(envelope['signature']),
        b'craft-apps-manager/app-release-catalog/v1\n' + payload)
    index = json.loads(payload)
    if (index['schema'] != 1 or index['purpose'] != 'app-release-catalog' or
            type(index['sequence']) is not int or index['sequence'] < 1 or
            str(index['sequence'])+'.json' != path.name or
            type(index['generated_at']) is not int or index['generated_at'] < 0):
        raise ValueError('Invalid retained root identity')
    return index


def newer(feed, published, source, tool):
    index = verified(tool, source, Path(published)/'index.json', Path(published)/'apps')
    previous = safe(feed, 'v1/index.json')
    if previous.exists():
        old = verified(tool, source, previous, safe(feed, 'v1/apps'))
        same = previous.read_bytes() == (Path(published)/'index.json').read_bytes()
        if index['sequence'] < old['sequence'] or (index['sequence'] == old['sequence'] and not same):
            raise ValueError('Catalog sequence rollback or conflicting publication')
    return index


def retention(feed, now):
    path = safe(feed, 'publisher/retention.json')
    ledger = read_json(path, 1024 * 1024) if path.exists() else dict(schema=1, retired={})
    if (set(ledger) != {'schema', 'retired'} or ledger['schema'] != 1 or
            len(ledger['retired']) > 10000 or any(not re.fullmatch('[0-9a-f]{64}', digest) or
            type(at) is not int or at < 0 or at > now for digest, at in ledger['retired'].items())):
        raise ValueError('Invalid catalog retention ledger')
    return path, ledger


def stage_objects(feed, published, source, tool, now=None):
    now = int(time.time()) if now is None else now
    index = newer(feed, published, source, tool)
    ledger_path, ledger = retention(feed, now)
    for reference in index['repositories'].values():
        name = reference['sha256'] + '.json'
        body = (Path(published)/'apps'/name).read_bytes()
        target = safe(feed, 'v1/apps/' + name)
        if target.exists():
            if target.read_bytes() != body:
                raise ValueError('Immutable app object has conflicting contents')
        else:
            write(target, body)
            # Track ownership before authority changes, so an interrupted first
            # phase cannot accumulate permanently unreferenced objects.
            ledger['retired'].setdefault(reference['sha256'], now)
    write(ledger_path, json.dumps(ledger, separators=(',', ':')).encode())


def stage_root(feed, published, candidate, source, tool, now=None):
    now = int(time.time()) if now is None else now
    index = newer(feed, published, source, tool)
    # Confirm the first publication phase completed before changing authority.
    verified(tool, source, Path(published)/'index.json', safe(feed, 'v1/apps'))
    bundle = read_json(Path(candidate)/'bundle.json')
    if bundle['index'] != index or set(bundle['objects']) != {r['sha256'] for r in index['repositories'].values()}:
        raise ValueError('Publisher checkpoint is not bound to the signed catalog')
    for digest, app in bundle['objects'].items():
        if app != read_json(Path(published)/'apps'/f'{digest}.json'):
            raise ValueError('Publisher object checkpoint does not match the signed catalog')
    checkpoint = {name: (Path(candidate)/name).read_bytes() for name in ['repositories.json','discovery.json','inspection.json','bundle.json']}
    status_path = Path(candidate)/'status.json'
    if status_path.exists():
        status = read_json(status_path, 4096)
        if (set(status) != {'schema', 'generated_at', 'pending_inspections'} or status['schema'] != 1 or
                type(status['pending_inspections']) is not int or not 0 <= status['pending_inspections'] <= 10000 or
                type(status['generated_at']) is not int or not 0 < status['generated_at'] <= now + 300):
            raise ValueError('Invalid publisher work status')
        checkpoint['status.json'] = status_path.read_bytes()
    for name in checkpoint:
        read_json(Path(candidate)/name)
    ledger_path, ledger = retention(feed, now)
    current = safe(feed, 'v1/index.json')
    old = None
    if current.exists():
        old = read_json(current, 256 * 1024)
        old_index = json.loads(old['payload'])
        archive = safe(feed, f"v1/roots/{old_index['sequence']}.json")
        if archive.exists() and archive.read_bytes() != current.read_bytes():
            raise ValueError('Conflicting retained catalog root')
        for reference in old_index['repositories'].values():
            ledger['retired'].setdefault(reference['sha256'], now)
    active = {ref['sha256'] for ref in index['repositories'].values()}
    for digest in active:
        ledger['retired'].pop(digest, None)
    # Preserve the previous root and all roots younger than the retention period.
    protected = set(active)
    if old:
        protected.update(ref['sha256'] for ref in old_index['repositories'].values())
    previous_sequence = json.loads(old['payload'])['sequence'] if old else None
    roots = safe(feed, 'v1/roots')
    expired_roots = []
    if roots.exists():
        keys = read_json(source, 64 * 1024)['keys']
        for path in roots.iterdir():
            if not re.fullmatch(r'[0-9]+\.json', path.name) or path.is_symlink() or not path.is_file():
                continue
            retained = retained_index(path, keys)
            if retained['sequence'] == previous_sequence or now - retained['generated_at'] < RETENTION_SECONDS:
                protected.update(ref['sha256'] for ref in retained['repositories'].values())
            else:
                expired_roots.append(path)
    expired_objects = []
    for digest, at in list(ledger['retired'].items()):
        if digest in protected or now - at < RETENTION_SECONDS:
            continue
        path = safe(feed, 'v1/apps/' + digest + '.json')
        if path.exists():
            if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
                raise ValueError('Retired catalog object has unrecognized contents')
            expired_objects.append(path)
        del ledger['retired'][digest]
    # Validate the complete cleanup plan first. A rejected archive or altered
    # object must leave the current authoritative index unchanged.
    if old:
        write(archive, current.read_bytes())
    for path in expired_roots + expired_objects:
        path.unlink()
    for name, body in checkpoint.items():
        destination = 'app-catalog/repositories.json' if name == 'repositories.json' else 'publisher/' + name
        write(safe(feed, destination), body)
    write(ledger_path, json.dumps(ledger, separators=(',', ':')).encode())
    write(current, (Path(published)/'index.json').read_bytes())


def collect(tool, source, previous, output, inspections):
    if os.environ.get('APP_RELEASE_CATALOG_SIGNING_KEY'):
        raise ValueError('Collection must run without a signing secret')
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    previous = Path(previous)
    old = previous/'publisher/bundle.json'
    approved = set(read_json(ROOT/'app-catalog/repositories.json'))
    if (previous/'v1/index.json').exists():
        index = verified(tool, source, previous/'v1/index.json', previous/'v1/apps')
        if read_json(old)['index'] != index:
            raise ValueError('Previous publisher checkpoint differs from its signed root')
        approved.update(index['repositories'])
    # A writable data branch cannot silently enroll unrelated repositories by
    # changing its unsigned checkpoint list. Only signed history and source
    # policy are retained; genuinely new apps pass the shared enrollment checks.
    seed = output/'repositories-seed.json'
    seed.write_text(json.dumps(sorted(approved)))
    try:
        invoke(tool, 'enroll', '--repositories', seed, '--out', output/'repositories.json')
    finally:
        seed.unlink()
    checkpoint = previous/'publisher/discovery.json'
    args = ['--checkpoint', checkpoint] if checkpoint.exists() else []
    invoke(tool, 'discover', '--repositories', output/'repositories.json', *args, '--out', output/'discovery.json')
    args = ['--previous', old] if old.exists() else []
    evidence = previous/'publisher/inspection.json'
    args += ['--checkpoint', evidence] if evidence.exists() else []
    invoke(tool, 'inspect-pending', '--discovery', output/'discovery.json', *args, '--max-packages', inspections,
           '--staging', output/'inspection-work', '--out', output/'inspection.json')
    args = ['--previous', old] if old.exists() else []
    invoke(tool, 'aggregate', '--discovery', output/'discovery.json', *args,
           '--evidence', output/'inspection.json', '--out', output/'bundle.json')
    pending = int(invoke(tool, 'pending-count', '--discovery', output/'discovery.json',
                         '--checkpoint', output/'inspection.json', '--previous', output/'bundle.json',
                         '--since', int(time.time()) - 12 * 3600))
    if not 0 <= pending <= 10000:
        raise ValueError('Inspection queue exceeds bounds')
    (output/'status.json').write_text(json.dumps(dict(schema=1, generated_at=int(time.time()), pending_inspections=pending)))


def baseline_file(feed, source, tool, output):
    feed = Path(feed)
    index = verified(tool, source, safe(feed, 'v1/index.json'), safe(feed, 'v1/apps'))
    value = dict(schema=1, envelope=safe(feed, 'v1/index.json').read_text(encoding='utf-8'),
        objects={ref['sha256']:safe(feed, 'v1/apps/'+ref['sha256']+'.json').read_text(encoding='utf-8')
                 for ref in index['repositories'].values()})
    output = Path(output)
    if output.is_symlink():
        raise ValueError('Linked baseline output')
    if output.exists() and set(read_json(output)) != {'schema','envelope','objects'}:
        raise ValueError('Refusing to replace an unrelated file with a baseline')
    write(output, json.dumps(value, separators=(',', ':')).encode())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['configuration','collect','stage-objects','stage-root','baseline'])
    parser.add_argument('--source', type=Path, default=ROOT/'assets/app-release-catalog/source.json')
    parser.add_argument('--tool', type=Path)
    parser.add_argument('--feed', type=Path)
    parser.add_argument('--published', type=Path)
    parser.add_argument('--candidate', type=Path)
    parser.add_argument('--out', type=Path)
    parser.add_argument('--inspections', type=int, default=8)
    args = parser.parse_args()
    if args.command == 'configuration':
        repo, branch, _ = configuration(args.source)
        print(json.dumps(dict(repository=repo, branch=branch)))
    elif args.command == 'collect':
        if not 0 <= args.inspections <= 32:
            parser.error('Inspection budget must be between 0 and 32')
        collect(args.tool, args.source, args.feed, args.candidate, args.inspections)
    elif args.command == 'stage-objects':
        stage_objects(args.feed, args.published, args.source, args.tool)
    elif args.command == 'baseline':
        if not args.out:
            parser.error('baseline requires --out')
        baseline_file(args.feed, args.source, args.tool, args.out)
    else:
        stage_root(args.feed, args.published, args.candidate, args.source, args.tool)


if __name__ == '__main__':
    main()
