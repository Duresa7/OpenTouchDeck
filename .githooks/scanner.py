#!/usr/bin/env python3
"""Fast, dependency-free checks for staged changes and outgoing commits."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
RULES = [
    ('private-key', r'-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----'),
    ('minisign-secret', r'\bRWR[A-Za-z0-9+/]{155}(?:==)?'),
    # base64("untrusted comment:") opens a wrapped key box; exempt() keeps only secret keys.
    ('minisign-secret-wrapped', r'\bdW50cnVzdGVkIGNvbW1lbnQ6[A-Za-z0-9+/]{200,}={0,2}'),
    ('companion-token', r'\btd[ps]_[A-Za-z0-9_-]{43}(?![A-Za-z0-9_-])'),
    ('github-token', r'\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})\b'),
    ('aws-access-key', r'\b(?:AKIA|ASIA|AIDA|AROA|AIPA|ANPA|ANVA|ASCA)[A-Z0-9]{16}\b'),
    ('email', r'[A-Za-z0-9.!#$%&\x27*+/=?^_`{|}~-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}(?![A-Za-z0-9-])'),
    ('windows-user-path', r'(?i)\b[A-Z]:(?:\\{1,2}|/)Users(?:\\{1,2}|/)([^\\/"\x27\r\n]+)(?:\\{1,2}|/)'),
    ('unix-user-path', r'/(?:home|Users)/([^/\s"\x27]+)/'),
]
RULES = [(name, re.compile(pattern)) for name, pattern in RULES]
SECRET_KEY = dict(RULES)['minisign-secret']


def git(*args, data=None):
    return subprocess.run(['git', *args], input=data, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, check=True).stdout


def mask(value):
    return value[:2] + '***'


def exempt(rule, match):
    if rule == 'minisign-secret-wrapped':
        try:
            # Comments may be any UTF-8 text; only the key line has to be ASCII.
            text = base64.b64decode(match.group() + '=' * (-len(match.group()) % 4)).decode('utf-8', 'replace')
        except ValueError:
            return True
        return not SECRET_KEY.search(text)
    if rule == 'email':
        local, domain = match.group().lower().rsplit('@', 1)
        return (domain in {'example.com', 'example.org', 'example.net', 'users.noreply.github.com'}
                or domain.endswith(('.example', '.test', '.invalid', '.localhost'))
                or local in {'noreply', 'no-reply'})
    if rule.endswith('user-path'):
        user = match.group(1).strip().lower()
        return (user.startswith('<') and user.endswith('>')
                or user.startswith('%') and user.endswith('%')
                or user.startswith('${') and user.endswith('}')
                or user in {'public', 'default', 'runneradmin', 'runner'})
    return False


def local_rules(common):
    path = common / 'info' / 'private-patterns'
    if not path.exists():
        return []
    rules = []
    for number, line in enumerate(path.read_text(encoding='utf-8').splitlines(), 1):
        line = line.strip()
        if not line or line.startswith('#'):
            continue
        try:
            pattern = re.compile(line[3:] if line.startswith('re:') else re.escape(line),
                                 0 if line.startswith('re:') else re.IGNORECASE)
        except re.error:
            raise ValueError(f'invalid local regex at entry {number}') from None
        rules.append((f'local-pattern-{number}', pattern))
    return rules


def allowlist():
    # Entries bind a rule to one exact line in one file, with a required explanation.
    entries = json.loads((HERE / 'allowlist.json').read_text(encoding='utf-8'))
    for entry in entries:
        if not entry.get('comment'):
            raise ValueError('allowlist entry needs a comment')
    return entries


def findings(path, number, line, rules, allowed):
    result = []
    for rule, pattern in rules:
        for match in pattern.finditer(line):
            if exempt(rule, match):
                continue
            if not rule.startswith('local-') and any(
                entry['file'] == path and entry['rule'] == rule
                and entry['line_sha256'] == hashlib.sha256(line.encode()).hexdigest()
                for entry in allowed
            ):
                continue
            result.append(f'{path}:{number}: {rule}: {mask(match.group())}')
    return result


def diff_lines(args):
    # NUL-delimited raw paths avoid Git's quoting rules, including for unusual filenames.
    paths = git('diff', '--no-color', '--name-only', '-z', '--diff-filter=ACMRT', *args).split(b'\0')
    for raw in filter(None, paths):
        path = os.fsdecode(raw)
        patch = git('diff', '--no-color', '--no-ext-diff', '--no-textconv', '--no-renames',
                    '--unified=0', *args, '--', path).decode('utf-8', 'replace')
        number = None
        for line in patch.splitlines():
            hunk = re.match(r'^@@ -\d+(?:,\d+)? \+(\d+)', line)
            if hunk:
                number = int(hunk.group(1))
            elif number is not None:
                if line.startswith('+'):
                    yield path, number, line[1:]
                    number += 1
                elif line.startswith(' '):
                    number += 1


def outgoing(ref_input):
    commits = set()
    for line in ref_input.splitlines():
        local_ref, local_sha, remote_ref, remote_sha = line.split()
        if set(local_sha) == {'0'}:
            continue
        # Tags may point to blobs/trees, which have no commits to scan.
        peeled = subprocess.run(['git', 'rev-parse', '--verify', local_sha + '^{commit}'],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if peeled.returncode:
            continue
        args = ['rev-list', peeled.stdout.decode().strip()]
        known = subprocess.run(['git', 'cat-file', '-e', remote_sha + '^{commit}'],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE).returncode == 0
        if set(remote_sha) != {'0'} and known:
            args.append('^' + remote_sha)
        else:
            # New branch, or a remote tip this clone never fetched: scan what no remote-tracking ref has.
            args.extend(['--not', '--remotes'])
        commits.update(git(*args).decode().splitlines())
    return sorted(commits)


def commit_lines(commit):
    message = git('show', '-s', '--format=%B', commit).decode('utf-8', 'replace')
    for number, line in enumerate(message.splitlines(), 1):
        yield f'commit {commit} message', number, line
    parents = git('rev-list', '--parents', '-n', '1', commit).decode().split()[1:]
    # Compare merges against every parent so additions hidden by combined diffs are checked.
    for parent in parents or [git('hash-object', '-t', 'tree', '--stdin', data=b'').decode().strip()]:
        yield from diff_lines([parent, commit])


def scan(lines, rules, allowed):
    count = 0
    for path, number, line in lines:
        for finding in findings(path, number, line, rules, allowed):
            print(finding, file=sys.stderr)
            count += 1
    if count:
        print(f'Blocked: {count} private-data finding(s). Remove the data and retry.', file=sys.stderr)
    return bool(count)


def all_lines():
    for raw in filter(None, git('ls-tree', '-rz', '--full-tree', 'HEAD').split(b'\0')):
        metadata, path = raw.split(b'\t', 1)
        mode, kind, oid = metadata.split()
        if kind != b'blob':
            continue
        content = git('cat-file', 'blob', oid.decode()).decode('utf-8', 'replace')
        for number, line in enumerate(content.splitlines(), 1):
            yield os.fsdecode(path), number, line


def chain(mode, args, common, ref_input):
    hook = common / 'hooks' / mode
    if hook.is_file() and os.access(hook, os.X_OK):
        if hook.resolve() == (HERE / mode).resolve():
            raise ValueError('local hook points back to committed hook')
        command = [str(hook), *args]
        if os.name == 'nt':
            command = ['sh', '-c', 'exec "$0" "$@"', *command]
        return subprocess.run(command, input=ref_input if mode == 'pre-push' else None).returncode
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--all-files', action='store_true')
    parser.add_argument('mode', nargs='?', choices=['pre-commit', 'pre-push', 'post-checkout'])
    parser.add_argument('hook_args', nargs='*')
    args = parser.parse_args()
    os.chdir(os.fsdecode(git('rev-parse', '--show-toplevel')).strip())
    common = Path(os.fsdecode(git('rev-parse', '--git-common-dir')).strip()).resolve()
    ref_input = sys.stdin.buffer.read() if args.mode == 'pre-push' else b''
    if args.mode == 'post-checkout' and not args.all_files:
        return chain(args.mode, args.hook_args, common, ref_input)
    rules = RULES + ([] if args.all_files else local_rules(common))
    allowed = allowlist()
    if args.all_files:
        failed = scan(all_lines(), rules, allowed)
        if not failed:
            print('Private-data scan: zero findings (all tracked files at HEAD).')
        return int(failed)
    if args.mode == 'pre-commit':
        if scan(diff_lines(['--cached']), rules, allowed):
            return 1
        staged = git('diff', '--cached', '--name-only', '-z').split(b'\0')
        if any(path.endswith(b'.rs') for path in staged):
            if shutil.which('cargo'):
                if subprocess.run(['cargo', 'fmt', '--check'], cwd='app').returncode:
                    return 1
            else:
                print('Note: cargo is missing; skipping Rust format check only.', file=sys.stderr)
    elif args.mode == 'pre-push':
        commits = outgoing(ref_input.decode())
        print(f'Private-data scan: {len(commits)} outgoing commit(s).')
        if scan((line for commit in commits for line in commit_lines(commit)), rules, allowed):
            return 1
    elif args.mode != 'post-checkout':
        parser.error('specify a hook or --all-files')
    return chain(args.mode, args.hook_args, common, ref_input)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.CalledProcessError):
        print('Hook failed: could not complete checks. Check Git, scanner configuration and Python 3.',
              file=sys.stderr)
        sys.exit(1)
