"""Publish the allowlisted documentation from an enabled main-branch workflow."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

from wiki import ROOT, REPOSITORY, OWNERSHIP, checked_path, generate, load_manifest, validate_preview
from wiki_sync import plan_changes, apply_changes

SOURCE_URL = 'https://github.com/' + REPOSITORY + '.git'
WIKI_URL = 'https://github.com/' + REPOSITORY + '.wiki.git'


def authorize(env):
    if (env.get('GITHUB_ACTIONS') != 'true' or env.get('GITHUB_REPOSITORY') != REPOSITORY
            or env.get('GITHUB_REF') != 'refs/heads/main'
            or env.get('GITHUB_EVENT_NAME') not in ('push', 'workflow_dispatch')
            or env.get('WIKI_SYNC_ENABLED') != 'true'):
        raise ValueError('Wiki publication requires an enabled main-branch workflow in the source repository')
    sha = env.get('GITHUB_SHA', '')
    if not re.fullmatch('[0-9a-f]{40}', sha):
        raise ValueError('Invalid source commit')
    if not env.get('WIKI_SYNC_TOKEN', '').strip():
        raise ValueError('Missing wiki-publication environment secret WIKI_SYNC_TOKEN')
    return sha


def sync_checkout(root, checkout, sha, git, ensure_current, bootstrap_hash=''):
    ensure_current()
    manifest = load_manifest(root / 'scripts/documentation/wiki-manifest.json')
    generated = generate(root, manifest, sha)
    validate_preview(generated, manifest)
    record = checked_path(checkout, OWNERSHIP)
    previous = json.loads(record.read_text(encoding='utf-8')) if record.is_file() else None
    adopt = set()
    if previous is None and bootstrap_hash:
        home = checked_path(checkout, 'Home.md')
        if not re.fullmatch('[0-9a-f]{64}', bootstrap_hash) or not home.is_file() or hashlib.sha256(home.read_bytes()).hexdigest() != bootstrap_hash:
            raise ValueError('Initial Home page differs from the explicitly approved bootstrap hash')
        adopt.add('Home.md')
    changes = plan_changes(checkout, generated, previous, adopt)
    if changes['conflicts']:
        raise ValueError('Wiki contains manual changes; reconcile source and wiki before publishing: ' + '; '.join(changes['conflicts']))
    if not changes['writes'] and not changes['deletes']:
        return False
    apply_changes(checkout, changes)
    git(checkout, 'add', '--', *sorted(set(changes['writes']) | set(changes['deletes'])))
    git(checkout, '-c', 'user.name=Documentation sync', '-c', 'user.email=docs-sync@users.noreply.github.com',
        'commit', '-m', 'Sync documentation from ' + sha)
    ensure_current()
    git(checkout, 'push', 'origin', 'HEAD')
    return True


def git_runner(env):
    def git(cwd, *args):
        result = subprocess.run(['git', '-c', 'credential.helper=', '-c', 'core.autocrlf=false',
                                 '-c', 'core.hooksPath=' + os.devnull, *args],
                                cwd=cwd, env=env, capture_output=True, text=True, timeout=180)
        if result.returncode:
            # Git diagnostics can contain credentials returned by a remote/helper.
            raise ValueError('Git operation failed (' + str(result.returncode) + '); no changes were force-pushed')
        return result.stdout.strip()
    return git


def main():
    sha = authorize(os.environ)
    # The production job uses Linux. Never persist credentials in a Git URL/config/file.
    if os.name != 'posix':
        raise ValueError('Wiki publisher runs only on the Linux workflow runner')
    with tempfile.TemporaryDirectory(prefix='craft-wiki-') as folder:
        temp = Path(folder)
        askpass = temp / 'askpass'
        askpass.write_text('#!' + sys.executable + '\nimport os, sys\n'
                           'print("x-access-token" if "username" in sys.argv[1].lower() else os.environ["WIKI_SYNC_TOKEN"])\n', encoding='utf-8')
        askpass.chmod(0o700)
        env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
        env.update(GIT_ASKPASS=str(askpass), GIT_TERMINAL_PROMPT='0', GIT_CONFIG_NOSYSTEM='1',
                   GIT_CONFIG_GLOBAL=os.devnull, GIT_ATTR_NOSYSTEM='1')
        git = git_runner(env)
        def ensure_current():
            if git(ROOT, 'rev-parse', 'HEAD') != sha or git(ROOT, 'status', '--porcelain', '--untracked-files=no'):
                raise ValueError('Source checkout must match the clean workflow commit')
            if git(ROOT, 'ls-remote', SOURCE_URL, 'refs/heads/main').split()[0] != sha:
                raise ValueError('Source main advanced; rerun publication from its current commit')
        ensure_current()
        refs = git(temp, 'ls-remote', '--symref', WIKI_URL, 'HEAD')
        branches = re.findall(r'^ref: (refs/heads/[^\s]+)\s+HEAD$', refs, re.M)
        if len(branches) != 1:
            raise ValueError('Initialize the wiki Home page before publication')
        branch = branches[0][len('refs/heads/'):]
        git(temp, 'check-ref-format', '--branch', branch)
        checkout = temp / 'wiki'
        git(temp, 'clone', '--single-branch', '--branch', branch, WIKI_URL, str(checkout))
        changed = sync_checkout(ROOT, checkout, sha, git, ensure_current, os.environ.get('WIKI_BOOTSTRAP_HOME_SHA256', ''))
        print('Wiki documentation published.' if changed else 'Wiki already matches this source commit; no commit or push.')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        print('Wiki publication stopped: ' + str(error), file=sys.stderr)
        sys.exit(1)
