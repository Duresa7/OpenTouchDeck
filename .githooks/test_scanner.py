"""Rule tests and real Git hook proofs; all suspect strings are built at runtime."""
import base64
import contextlib
import hashlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import scanner


def token():
    return 'tdp' + '_' + 'x' * 43


def wrapped(second_line, comment='fixture'):
    # The release signer's box: base64 of an untrusted comment and the key or signature line.
    return base64.b64encode(('untrusted comment: ' + comment + '\n' + second_line + '\n').encode()).decode()


class Rules(unittest.TestCase):
    def matches(self, text, rules=None, allowed=None):
        return scanner.findings('fixture', 7, text, rules or scanner.RULES, allowed or [])

    def test_each_rule(self):
        samples = {
            'private-key': ['-----BEGIN ' + kind + 'PRIVATE KEY-----' for kind in ('', 'RSA ', 'OPENSSH ', 'EC ', 'ENCRYPTED ')],
            'minisign-secret': ['RW' + 'R' + 'a' * 209],
            'minisign-secret-wrapped': [wrapped('RW' + 'RTY0Iy' + 'A' * 204 + '='),
                                        wrapped('RW' + 'RTY0Iy' + 'A' * 204 + '=', 'Cl\u00e9 priv\u00e9e')],
            'companion-token': [token(), 'tds' + '_' + 'a' * 43],
            'github-token': [prefix + '_' + 'a' * 36 for prefix in ('ghp', 'gho', 'ghu', 'ghs', 'ghr', 'github_pat')],
            'aws-access-key': ['AK' + 'IA' + 'A' * 16, 'AS' + 'IA' + 'B' * 16],
            'email': ['someone' + '@' + 'private-domain.dev'],
            'windows-user-path': ['C:' + '\\Users\\' + 'fake-person' + '\\data',
                                  '"C:' + '\\\\Users\\\\' + 'fake-person' + '\\\\data"',
                                  'C:' + '\\Users\\' + 'Fake Person' + '\\data'],
            'unix-user-path': ['/' + base + '/' + 'fake-person' + '/data' for base in ('home', 'Users')],
        }
        for rule, values in samples.items():
            for value in values:
                with self.subTest(rule=rule, value=value):
                    self.assertTrue(any(rule in result for result in self.matches(value)))

    def test_placeholders_and_private_ips(self):
        values = ['u' + '@' + domain for domain in ('example.com', 'example.org', 'example.net', 'x.example', 'x.test', 'x.invalid', 'x.localhost', 'users.noreply.github.com')]
        values += ['noreply' + '@' + 'anthropic.com', '192.' + '168.1.2']
        values += ['C:' + '\\Users\\' + name + '\\data' for name in ('<user>', '%USERNAME%', '%USERPROFILE%', 'Public', 'Default', 'runneradmin')]
        values += ['/' + base + '/' + name + '/data' for base in ('home', 'Users') for name in ('<user>', '${USER}', 'runner')]
        for value in values:
            self.assertEqual([], self.matches(value), value)
        self.assertEqual([], self.matches('tdp' + '_' + 'x' * 42))
        # Public key and signature boxes are published on purpose.
        self.assertEqual([], self.matches(wrapped('RW' + 'Q' + 'b' * 53) + 'A' * 160))

    def test_allowlist_is_exact_and_never_applies_to_local_patterns(self):
        value = token()
        allowed = [dict(file='fixture', rule='companion-token', line_sha256=hashlib.sha256(value.encode()).hexdigest())]
        self.assertEqual([], self.matches(value, allowed=allowed))
        self.assertTrue(self.matches(value + ' changed', allowed=allowed))
        allowed[0]['file'] = 'elsewhere'
        self.assertTrue(self.matches(value, allowed=allowed))
        local = [('local-pattern-1', scanner.re.compile(scanner.re.escape(value)))]
        self.assertTrue(self.matches(value, rules=local, allowed=allowed))

    def test_local_patterns(self):
        with tempfile.TemporaryDirectory() as directory:
            common = Path(directory)
            (common / 'info').mkdir()
            self.assertEqual([], scanner.local_rules(common))
            path = common / 'info/private-patterns'
            path.write_text('# comment\n\nFakeLiteral\nre:fake[0-9]+\n')
            rules = scanner.local_rules(common)
            self.assertTrue(self.matches('FAKELITERAL', rules=rules))
            self.assertTrue(self.matches('fake42', rules=rules))
            self.assertEqual([], self.matches('fakeXX', rules=rules))
            path.write_text('re:[\n')
            with self.assertRaises(ValueError):
                scanner.local_rules(common)

    def test_mask_and_diagnostics(self):
        self.assertEqual('ab***', scanner.mask('abcdef'))
        output = io.StringIO()
        with contextlib.redirect_stderr(output):
            self.assertTrue(scanner.scan([('fixture', 7, token())], scanner.RULES, []))
        self.assertIn('fixture:7: companion-token: td***', output.getvalue())
        self.assertNotIn(token(), output.getvalue())


class GitHooks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.remote = self.root / 'remote.git'
        self.repo = self.root / 'clone'
        self.run_git(self.root, 'init', '--bare', str(self.remote))
        seed = self.root / 'seed'
        self.run_git(self.root, 'init', str(seed))
        self.configure(seed)
        (seed / 'old.txt').write_text(token())
        self.run_git(seed, 'add', '.')
        self.run_git(seed, 'commit', '-m', 'test: historical fixture')
        self.run_git(seed, 'remote', 'add', 'origin', str(self.remote))
        self.run_git(seed, 'push', 'origin', 'HEAD:main')
        self.run_git(self.root, 'clone', '-b', 'main', str(self.remote), str(self.repo))
        self.configure(self.repo)
        shutil.copytree(scanner.HERE, self.repo / '.githooks', ignore=shutil.ignore_patterns('__pycache__'))
        self.git('config', 'core.hooksPath', '.githooks')
        self.git('add', '.githooks')
        self.git('commit', '-m', 'build: add hooks')
        self.git('push', 'origin', 'main')

    def run_git(self, cwd, *args, ok=True, data=None):
        result = subprocess.run(['git', *args], cwd=cwd, input=data, text=True, capture_output=True)
        if ok:
            self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        return result

    def git(self, *args, **kwargs):
        return self.run_git(self.repo, *args, **kwargs)

    def configure(self, repo):
        self.run_git(repo, 'config', 'user.name', 'Hook Test')
        self.run_git(repo, 'config', 'user.email', 'hooks' + '@example.com')

    def commit(self, name, text, message='test: clean fixture'):
        (self.repo / name).write_text(text)
        self.git('add', name)
        return self.git('commit', '-m', message, ok=False)

    def test_commit_blocks_token_and_windows_path_then_clean_passes(self):
        result = self.commit('fixture.txt', token() + '\nC:' + '\\Users\\' + 'fake-person' + '\\data\n')
        self.assertNotEqual(0, result.returncode)
        self.assertIn('companion-token', result.stderr)
        self.assertIn('windows-user-path', result.stderr)
        self.assertEqual(0, self.commit('fixture.txt', 'clean\n').returncode)

    @unittest.skipUnless(shutil.which('cargo'), 'requires Cargo for real Rust formatting proof')
    def test_misformatted_rust_is_blocked(self):
        app = self.repo / 'app'
        (app / 'src').mkdir(parents=True)
        (app / 'Cargo.toml').write_text('[package]\nname="hook-fixture"\nversion="0.1.0"\nedition="2021"\n')
        self.git('add', 'app/Cargo.toml')
        result = self.commit('app/src/main.rs', 'fn main(){println!("fixture");}\n')
        self.assertNotEqual(0, result.returncode)
        self.assertIn('Diff in', result.stdout + result.stderr)

    def test_local_pattern_blocks_push_and_preserves_stdin_for_chain(self):
        self.assertEqual(0, self.commit('fixture.txt', 'fake-local-marker\n').returncode)
        (self.repo / '.git/info/private-patterns').write_text('FAKE-local-marker\n')
        result = self.git('push', 'origin', 'main', ok=False)
        self.assertNotEqual(0, result.returncode)
        self.assertIn('local-pattern-1', result.stderr)
        (self.repo / '.git/info/private-patterns').unlink()
        hook = self.repo / '.git/hooks/pre-push'
        hook.write_text('#!/bin/sh\ncat > .git/push-input\nprintf "%s\\n" "$@" > .git/push-args\n')
        hook.chmod(0o755)
        self.git('push', 'origin', 'main')
        self.assertIn('refs/heads/main', (self.repo / '.git/push-input').read_text())
        self.assertEqual('origin', (self.repo / '.git/push-args').read_text().splitlines()[0])

    def test_new_branch_only_scans_new_commits(self):
        self.git('checkout', '-b', 'topic')
        self.assertEqual(0, self.commit('fixture.txt', 'clean\n').returncode)
        result = self.git('push', 'origin', 'topic')
        self.assertIn('1 outgoing commit(s)', result.stdout + result.stderr)
        sha = self.git('rev-parse', 'HEAD').stdout.strip()
        with patch('scanner.git', side_effect=lambda *args, **kw: subprocess.check_output(['git', *args], cwd=self.repo, input=kw.get('data'))):
            previous = os.getcwd()
            try:
                os.chdir(self.repo)
                self.assertEqual([], scanner.outgoing('refs/heads/topic ' + '0' * 40 + ' refs/heads/topic ' + sha))
            finally:
                os.chdir(previous)

    def test_forced_colour_still_scans_added_lines(self):
        self.git('config', 'color.ui', 'always')
        result = self.commit('fixture.txt', token() + '\n')
        self.assertNotEqual(0, result.returncode)
        self.assertIn('companion-token', result.stderr)

    def test_force_push_over_an_unfetched_remote_tip(self):
        other = self.root / 'other'
        self.run_git(self.root, 'clone', '-b', 'main', str(self.remote), str(other))
        self.configure(other)
        (other / 'other.txt').write_text('clean\n')
        self.run_git(other, 'add', 'other.txt')
        self.run_git(other, 'commit', '-m', 'test: advance the remote')
        self.run_git(other, 'push', 'origin', 'main')
        self.assertEqual(0, self.commit('fixture.txt', 'clean\n').returncode)
        result = self.git('push', '--force', 'origin', 'main', ok=False)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn('1 outgoing commit(s)', result.stdout + result.stderr)

    def test_commit_message_is_scanned(self):
        self.assertEqual(0, self.commit('fixture.txt', 'clean\n', 'test: ' + token()).returncode)
        result = self.git('push', 'origin', 'main', ok=False)
        self.assertNotEqual(0, result.returncode)
        self.assertIn('message:1: companion-token', result.stderr)

    def test_staged_content_only_and_local_pre_commit_chain(self):
        hook = self.repo / '.git/hooks/pre-commit'
        hook.write_text('#!/bin/sh\nprintf chained > .git/commit-proof\n')
        hook.chmod(0o755)
        (self.repo / 'fixture.txt').write_text('clean\n')
        self.git('add', 'fixture.txt')
        (self.repo / 'fixture.txt').write_text(token())
        self.git('commit', '-m', 'test: staged content')
        self.assertEqual('chained', (self.repo / '.git/commit-proof').read_text())

    def test_existing_ref_scans_intermediate_commits(self):
        self.assertEqual(0, self.commit('fixture.txt', 'clean\n', 'test: ' + token()).returncode)
        self.assertEqual(0, self.commit('fixture.txt', 'clean again\n').returncode)
        result = self.git('push', 'origin', 'main', ok=False)
        self.assertNotEqual(0, result.returncode)
        self.assertIn('2 outgoing commit(s)', result.stdout + result.stderr)
        self.assertIn('message:1: companion-token', result.stderr)

    def test_linked_worktree_uses_common_local_hook(self):
        hook = self.repo / '.git/hooks/post-checkout'
        hook.write_text('#!/bin/sh\nprintf chained > checkout-proof\n')
        hook.chmod(0o755)
        worktree = self.root / 'worktree'
        self.git('worktree', 'add', '-b', 'worktree-proof', str(worktree))
        self.assertEqual('chained', (worktree / 'checkout-proof').read_text())

    def test_local_post_checkout_chain_and_failure(self):
        hook = self.repo / '.git/hooks/post-checkout'
        hook.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > .git/checkout-proof\nexit 9\n')
        hook.chmod(0o755)
        result = self.git('checkout', '-b', 'chain-proof', ok=False)
        self.assertNotEqual(0, result.returncode)
        self.assertEqual('1', (self.repo / '.git/checkout-proof').read_text().splitlines()[-1])


if __name__ == '__main__':
    unittest.main()
