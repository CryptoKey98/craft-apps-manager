"""Select CI work from the complete change set and enforce its final result."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
CORE_JOBS = {'format', 'updater-v2', 'windows', 'linux', 'macos'}
PACKAGE_JOBS = {'windows-packages', 'linux-packages', 'ubuntu-deb', 'debian', 'fedora', 'archlinux', 'macos-packages'}
ALL_JOBS = {'changes', 'documentation', 'release-metadata', 'production-catalog'} | CORE_JOBS | PACKAGE_JOBS
FLAGS = ('core', 'packages', 'metadata', 'catalog')


def classify(paths, mode='changed'):
    if mode not in ('changed', 'full', 'catalog'):
        raise ValueError('Unknown CI mode')
    flags = dict.fromkeys(FLAGS, False)
    if mode == 'catalog':
        flags['catalog'] = True
        return flags
    if mode == 'full' or not paths:
        return dict(core=True, packages=True, metadata=True, catalog=False)
    for path in paths:
        if path == 'codemagic.yaml':
            continue
        if path == 'CHANGELOG.md':
            flags['metadata'] = True
            continue
        if path in ('README.md', 'LICENSE', 'THIRD-PARTY-NOTICES.txt') or (
                path.startswith('docs/') and (path.endswith('.md') or path.startswith('docs/images/'))):
            continue
        flags['core'] = flags['metadata'] = True
        # Ordinary UI/catalog changes need platform tests. Installation, process,
        # storage, dependency and unknown changes also need package transactions.
        ordinary = path.startswith(('src/ui/', 'src/catalog/', 'src/scheduling/'))
        if not ordinary:
            flags['packages'] = True
    return flags


def required_jobs(flags):
    required = {'changes'}
    if flags['catalog']:
        return required | {'production-catalog'}
    required.add('documentation')
    if flags['core']:
        required |= CORE_JOBS
    if flags['metadata']:
        required.add('release-metadata')
    if flags['packages']:
        required |= PACKAGE_JOBS
    return required


def verify_gate(needs):
    if not isinstance(needs, dict) or needs.get('changes', {}).get('result') != 'success':
        raise ValueError('Change detection did not succeed')
    outputs = needs['changes'].get('outputs', {})
    if any(outputs.get(key) not in ('true', 'false') for key in FLAGS):
        raise ValueError('Missing or invalid change-detection output')
    flags = {key: outputs[key] == 'true' for key in FLAGS}
    if (flags['packages'] and not (flags['core'] and flags['metadata'])) or (
            flags['core'] and not flags['metadata']) or (
            flags['catalog'] and any(flags[key] for key in ('core', 'packages', 'metadata'))):
        raise ValueError('Inconsistent change-detection output')
    failed = sorted(name for name in required_jobs(flags) if needs.get(name, {}).get('result') != 'success')
    failed += sorted(name for name, job in needs.items() if job.get('result') in ('failure', 'cancelled') and name not in failed)
    if failed:
        raise ValueError('Required CI work did not succeed: ' + ', '.join(failed))


def event_base(event, payload):
    if event == 'workflow_dispatch':
        return None
    if event == 'pull_request':
        base = payload.get('pull_request', {}).get('base', {}).get('sha', '')
    elif event == 'push':
        base = payload.get('before', '')
    else:
        raise ValueError('Unsupported CI event')
    if not isinstance(base, str) or not re.fullmatch('[0-9a-f]{40}', base):
        raise ValueError('Missing or invalid base commit')
    return None if base == '0' * 40 else base


def changed_paths(root, base, head):
    if any(not re.fullmatch('[0-9a-f]{40}', value) for value in (base, head)):
        raise ValueError('Invalid diff commit')
    result = subprocess.run(['git', 'diff', '--no-renames', '--name-only', '-z', base, head, '--'],
                            cwd=root, capture_output=True, check=True)
    return [path.decode('utf-8') for path in result.stdout.split(b'\0') if path]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('plan', 'gate'))
    args = parser.parse_args()
    if args.command == 'gate':
        verify_gate(json.loads(os.environ['CI_NEEDS']))
        print('All requested CI checks passed.')
        return
    event = os.environ['GITHUB_EVENT_NAME']
    payload = json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text(encoding='utf-8'))
    base = event_base(event, payload)
    catalog = payload.get('inputs', {}).get('production_catalog_only') in (True, 'true')
    mode = 'catalog' if event == 'workflow_dispatch' and catalog else ('full' if base is None else 'changed')
    paths = changed_paths(ROOT, base, os.environ['GITHUB_SHA']) if base is not None else []
    flags = classify(paths, mode)
    with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as output:
        for name, value in flags.items():
            output.write(f'{name}={str(value).lower()}\n')
    print('CI plan: ' + json.dumps(flags, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        print('CI policy failed: ' + str(error), file=sys.stderr)
        sys.exit(1)
