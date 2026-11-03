"""Checksum-bound, exact-identity executable cache for the catalog workflow."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

MAX_BINARY = 64 * 1024 * 1024


def read(path, limit):
    path = Path(path)
    if path.is_symlink() or path.parent.is_symlink():
        raise ValueError('Linked cache files are not supported')
    with path.open('rb') as stream:
        body = stream.read(limit + 1)
    if len(body) > limit:
        raise ValueError('Oversized publisher cache file')
    return body


def write(path, body):
    path = Path(path)
    if path.is_symlink() or path.parent.is_symlink():
        raise ValueError('Linked cache output is not supported')
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(dir=path.parent, prefix='.publisher-')
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(body)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def restore(directory, output, identity):
    directory = Path(directory)
    try:
        receipt = json.loads(read(directory/'receipt.json', 4096))
        if (not isinstance(receipt, dict) or
                set(receipt) != {'schema', 'identity', 'size', 'sha256'} or
                type(receipt['schema']) is not int or receipt['schema'] != 1 or receipt['identity'] != identity or
                type(receipt['size']) is not int or not 0 < receipt['size'] <= MAX_BINARY):
            return False
        body = read(directory/'tool.exe', MAX_BINARY)
        if (not body.startswith(b'MZ') or len(body) != receipt['size'] or
                hashlib.sha256(body).hexdigest() != receipt['sha256']):
            return False
    except (OSError, ValueError, UnicodeError, RecursionError):
        return False
    write(output, body)
    return True


def store(directory, source, identity):
    body = read(source, MAX_BINARY)
    if not body.startswith(b'MZ') or not identity or len(identity) > 1024:
        raise ValueError('Invalid publisher executable or cache identity')
    directory = Path(directory)
    receipt = dict(schema=1, identity=identity, size=len(body), sha256=hashlib.sha256(body).hexdigest())
    write(directory/'tool.exe', body)
    write(directory/'receipt.json', json.dumps(receipt, sort_keys=True).encode())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['restore', 'store'])
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--identity', required=True)
    args = parser.parse_args()
    if args.command == 'store':
        store(args.directory, args.binary, args.identity)
        print('Saved publisher executable and checksum receipt.')
    else:
        reused = restore(args.directory, args.binary, args.identity)
        if os.environ.get('GITHUB_OUTPUT'):
            with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as stream:
                stream.write(f'reused={str(reused).lower()}\n')
        if reused:
            print('Reusing verified publisher executable.')
        elif args.directory.exists():
            print('Invalid publisher cache; rebuilding. Delete this exact Actions cache entry if misses repeat.')
        else:
            print('Publisher cache miss; a build is required.')


if __name__ == '__main__':
    main()
