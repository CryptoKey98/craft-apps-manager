"""Generate a local, allowlisted wiki preview. This module cannot publish."""
import argparse
import hashlib
import json
import posixpath
import html
from pathlib import Path, PurePosixPath
import re
import sys
from urllib.parse import quote, unquote, urlsplit

REPOSITORY = 'CryptoKey98/craft-apps-manager'
WIKI = 'https://github.com/' + REPOSITORY + '/wiki'
OWNERSHIP = '.craft-docs-sync.json'
GENERATED = '<!-- Generated from repository documentation; edit the source file. -->'
ROOT = Path(__file__).resolve().parents[2]


def relative(value):
    if not isinstance(value, str) or not value or any(c in value for c in '\\:?#\r\n\x00'):
        raise ValueError('Unsafe relative path: ' + repr(value))
    path = PurePosixPath(value)
    if path.is_absolute() or any(p in ('', '.', '..') for p in value.split('/')):
        raise ValueError('Unsafe relative path: ' + value)
    return path


def linked(path):
    try:
        info = path.lstat()
    except FileNotFoundError:
        return False
    return path.is_symlink() or bool(getattr(info, 'st_file_attributes', 0) & 0x400)


def checked_path(root, name):
    path = root
    if linked(root):
        raise ValueError('Linked root is not permitted')
    for part in relative(name).parts:
        path = path / part
        if linked(path):
            raise ValueError('Linked path is not permitted: ' + name)
    if not path.resolve().is_relative_to(root.resolve()):
        raise ValueError('Path escapes root')
    return path


def validate_manifest(manifest):
    if manifest.get('schema') != 1 or manifest.get('repository') != REPOSITORY:
        raise ValueError('Unexpected wiki manifest scope')
    sources, destinations = set(), {'_sidebar.md', '_footer.md', OWNERSHIP}
    for kind in ('pages', 'assets'):
        if not isinstance(manifest.get(kind), list) or not 1 <= len(manifest[kind]) <= 128:
            raise ValueError('Invalid manifest list')
        for item in manifest[kind]:
            source, dest = str(relative(item['source'])), str(relative(item['destination']))
            public = (source in ('README.md', 'CHANGELOG.md', 'docs/README.md', 'update-service/catalog/README.md')
                      or source.startswith(('docs/user-guide/', 'docs/development/', 'docs/architecture/')))
            if kind == 'pages':
                if not public or not source.endswith('.md') or '/' in dest or not re.fullmatch(r'[A-Za-z0-9-]+\.md', dest):
                    raise ValueError('Not an approved public documentation path')
                if any(not isinstance(item.get(k), str) or not re.fullmatch(r'[A-Za-z0-9 /-]+', item[k]) for k in ('title', 'group')):
                    raise ValueError('Invalid navigation title/group')
            elif not ((source == 'assets/icon.png' or source.startswith('docs/images/')) and source.endswith('.png') and re.fullmatch(r'images/[A-Za-z0-9-]+\.png', dest)):
                raise ValueError('Not an approved image path')
            if any(part.startswith('.') for part in relative(source).parts):
                raise ValueError('Private path is not permitted')
            if source.casefold() in sources or dest.casefold() in destinations:
                raise ValueError('Duplicate source/destination')
            sources.add(source.casefold())
            destinations.add(dest.casefold())
    for alias, target in manifest.get('aliases', {}).items():
        relative(alias)
        if not alias.startswith('docs/') or target.casefold() not in sources:
            raise ValueError('Invalid legacy alias')
    if manifest.get('source_links') != ['LICENSE', 'THIRD-PARTY-NOTICES.txt']:
        raise ValueError('Unexpected non-page export links')


def load_manifest(path):
    manifest = json.loads(path.read_text(encoding='utf-8'))
    validate_manifest(manifest)
    return manifest


def rewrite_markdown(text, source, manifest, source_commit):
    mapping = {p['source']: p['destination'] for p in manifest['pages'] + manifest['assets']}
    def resolve(value):
        value = html.unescape(value)
        parsed = urlsplit(re.sub(r'\\([() ])', r'\1', value))
        if parsed.scheme or parsed.netloc:
            if parsed.scheme not in ('https', 'http', 'mailto'):
                raise ValueError('Unsupported URL scheme in ' + source)
            return value
        path = unquote(parsed.path)
        if path.startswith('/') or '\\' in path:
            raise ValueError('Absolute local reference in ' + source)
        target = posixpath.normpath(posixpath.join(posixpath.dirname(source), path)) if path else source
        target = manifest.get('aliases', {}).get(target, target)
        if target in mapping:
            dest = mapping[target]
            base = ('https://raw.githubusercontent.com/wiki/' + REPOSITORY + '/' + quote(dest, safe='/')
                    if dest.startswith('images/') else page_url(dest))
        elif target in manifest['source_links']:
            base = 'https://github.com/' + REPOSITORY + '/blob/' + source_commit + '/' + quote(target, safe='/')
        else:
            raise ValueError('Unresolved local reference in ' + source + ': ' + value)
        return base + ('?' + parsed.query if parsed.query else '') + ('#' + parsed.fragment if parsed.fragment else '')
    edits = [(a, b, resolve(text[a:b])) for a, b in link_spans(text)]
    for a, b, value in sorted(edits, reverse=True):
        text = text[:a] + value + text[b:]
    return text


def code_mask(text, mask_inline=True):
    """Same-length text with fenced/indented and inline code hidden from link scanning."""
    chars = list(text)
    offset, fence = 0, None
    for line in text.splitlines(keepends=True):
        marker = re.match(r'^ {0,3}(`{3,}|~{3,})', line)
        protected = fence is not None or marker is not None or line.startswith(('    ', '\t'))
        if fence:
            if re.fullmatch(r' {0,3}' + re.escape(fence[0]) + '{' + str(fence[1]) + r',}\s*', line):
                fence = None
        elif marker:
            fence = (marker[1][0], len(marker[1]))
        if protected:
            chars[offset:offset + len(line)] = ['\n' if c == '\n' else ' ' for c in line]
        offset += len(line)
    masked = ''.join(chars)
    if not mask_inline:
        return masked
    i = 0
    while i < len(masked):
        if masked[i] == '`':
            run = len(masked[i:]) - len(masked[i:].lstrip('`'))
            end = re.search(r'(?<!`)`{' + str(run) + r'}(?!`)', masked[i + run:])
            if end:
                stop = i + run + end.end()
                chars[i:stop] = ['\n' if c == '\n' else ' ' for c in masked[i:stop]]
                i = stop
                continue
            i += run
        else:
            i += 1
    return ''.join(chars)


def link_spans(text):
    """Locate destinations, retaining original labels, titles and code byte-for-byte."""
    masked = code_mask(text)
    spans = set()
    # Reference definitions. The destination may be bracketed to contain spaces.
    for match in re.finditer(r'^ {0,3}\[[^\]\n]+\]:[ \t]*(?:<([^>\n]+)>|([^\s]+))', masked, re.M):
        group = 1 if match[1] is not None else 2
        spans.add(match.span(group))
    # Markdown inline links, including images nested in linked labels.
    for match in re.finditer(r'(?<!\\)\[', masked):
        i, depth = match.end(), 1
        while i < len(masked) and depth:
            if masked[i] == '\\':
                i += 2
                continue
            depth += (masked[i] == '[') - (masked[i] == ']')
            i += 1
        if depth or i >= len(masked) or masked[i] != '(':
            continue
        i += 1
        while i < len(masked) and masked[i].isspace():
            i += 1
        start = i + (i < len(masked) and masked[i] == '<')
        if i < len(masked) and masked[i] == '<':
            end = masked.find('>', start)
            if end < 0:
                raise ValueError('Unclosed Markdown destination')
        else:
            depth, end = 0, i
            while end < len(masked):
                c = masked[end]
                if c == '\\':
                    end += 2
                    continue
                if (c == ')' and depth == 0) or (c.isspace() and depth == 0):
                    break
                depth += (c == '(') - (c == ')')
                end += 1
        if end > start:
            spans.add((start, end))
    for tag in re.finditer(r'<(?:img|a)\b[^>]*>', masked, re.I):
        for attr in re.finditer(r'\b(?:src|href)\s*=\s*([\'"])(.*?)\1', tag[0], re.I):
            spans.add((tag.start() + attr.start(2), tag.start() + attr.end(2)))
    ordered = sorted(spans)
    if any(a[1] > b[0] for a, b in zip(ordered, ordered[1:])):
        raise ValueError('Overlapping Markdown destinations')
    return ordered


def anchors(text):
    seen, values = {}, set()
    for heading in re.findall(r'^ {0,3}#{1,6}\s+(.+?)\s*#*$', code_mask(text, mask_inline=False), re.M):
        slug = re.sub(r'[^\w -]', '', re.sub(r'<[^>]*>', '', html.unescape(heading).lower())).replace(' ', '-')
        n = seen.get(slug, 0)
        candidate = slug + (f'-{n}' if n else '')
        while candidate in values:
            n += 1
            candidate = slug + f'-{n}'
        seen[slug] = n + 1
        values.add(candidate)
    return values


def validate_preview(files, manifest):
    pages = {p['destination'][:-3]: p['destination'] for p in manifest['pages']}
    for name, body in files.items():
        if not name.endswith('.md'):
            continue
        text = body.decode('utf-8')
        for a, b in link_spans(text):
            value = html.unescape(text[a:b])
            url = urlsplit(value)
            if value.startswith(WIKI + '/'):
                title = unquote(url.path.split('/wiki/', 1)[1])
                dest = pages.get(title)
                if dest is None or dest not in files:
                    raise ValueError('Unknown wiki target in ' + name + ': ' + value)
                if url.fragment and unquote(url.fragment) not in anchors(files[dest].decode('utf-8')):
                    raise ValueError('Unknown heading in ' + name + ': ' + value)
            elif value.startswith('https://raw.githubusercontent.com/wiki/' + REPOSITORY + '/'):
                dest = unquote(url.path.split('/' + REPOSITORY + '/', 1)[1])
                if dest not in files:
                    raise ValueError('Missing wiki image: ' + value)
            elif not url.scheme:
                raise ValueError('Untranslated local link in ' + name + ': ' + value)


def page_url(destination):
    return WIKI + '/' + quote(destination[:-3], safe='-')


def ownership(files, source_commit):
    return json.dumps({'schema': 1, 'repository': REPOSITORY, 'source_commit': source_commit,
                       'files': {n: hashlib.sha256(b).hexdigest() for n, b in sorted(files.items())}}, indent=2).encode() + b'\n'


def generate(root, manifest, source_commit):
    validate_manifest(manifest)
    if not re.fullmatch(r'[0-9a-f]{40}', source_commit):
        raise ValueError('Use a full 40-character source commit SHA')
    root = Path(root).absolute()
    files = {}
    for item in manifest['pages'] + manifest['assets']:
        source = checked_path(root, item['source'])
        if not source.is_file():
            raise ValueError('Missing source: ' + item['source'])
        body = source.read_bytes()
        if source.suffix == '.md':
            text = rewrite_markdown(body.decode('utf-8'), item['source'], manifest, source_commit)
            url = 'https://github.com/' + REPOSITORY + '/blob/' + source_commit + '/' + quote(item['source'], safe='/')
            body = (GENERATED + '\n\n' + text.rstrip() + '\n\n---\n\n[Edit this page in the source repository](' + url + ')\n').encode()
        files[item['destination']] = body
    sidebar, group = [], None
    for item in manifest['pages']:
        if item['group'] != group:
            group = item['group']
            sidebar.extend(['', '**' + group + '**', ''])
        sidebar.append('- [' + item['title'] + '](' + page_url(item['destination']) + ')')
    files['_Sidebar.md'] = ('\n'.join(sidebar).strip() + '\n').encode()
    files['_Footer.md'] = ('Documentation is generated from the [source repository](https://github.com/' + REPOSITORY + '). Edit the original Markdown, not the wiki.\n').encode()
    files[OWNERSHIP] = ownership(files, source_commit)
    return files


def prepare(root, manifest, source_commit, output):
    root, output = Path(root).absolute(), Path(output).absolute()
    if output != root / 'target/wiki-preview':
        raise ValueError('Preview output must be target/wiki-preview')
    checked_path(root, 'target/wiki-preview')
    files = generate(root, manifest, source_commit)
    validate_preview(files, manifest)
    output.mkdir(parents=True, exist_ok=True)
    from wiki_sync import plan_changes, apply_changes, validate_ownership
    prior_path = checked_path(output, OWNERSHIP)
    prior = json.loads(prior_path.read_text(encoding='utf-8')) if prior_path.is_file() else None
    if prior is not None:
        validate_ownership(prior)
    allowed = set(files) | (set(prior['files']) if prior else set())
    for path in output.rglob('*'):
        name = path.relative_to(output).as_posix()
        checked_path(output, name)
        if path.is_file() and name not in allowed:
            raise ValueError('Unlisted file in preview; left untouched: ' + name)
    apply_changes(output, plan_changes(output, files, prior, set()))
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('prepare', 'check'))
    parser.add_argument('--source-commit', required=True)
    args = parser.parse_args()
    manifest = load_manifest(ROOT / 'scripts/documentation/wiki-manifest.json')
    files = prepare(ROOT, manifest, args.source_commit, ROOT / 'target/wiki-preview')
    print(f'Prepared {len(files)} local wiki files; nothing published.')


if __name__ == '__main__':
    main()
