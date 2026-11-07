import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = ROOT / '.github/workflows/release-windows.yml'
NAME = 'OpenTouchDeck-windows-x64-setup.exe'
RADIO = 'touchdeck-radio-1.0.0.tdfw'


def workflow_shell(step):
    source = WORKFLOW.read_text(encoding='utf-8').split(f'      - name: {step}\n', 1)[1]
    lines = source.split('        run: |\n', 1)[1].splitlines()
    body = []
    for line in lines:
        if line.strip() and not line.startswith('          '):
            break
        body.append(line[10:])
    return '\n'.join(body)


@unittest.skipUnless(shutil.which('bash') and shutil.which('jq'), 'bash and jq required')
class ReleaseWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.assets = self.root / 'release-assets'
        self.assets.mkdir()
        (self.root / 'signed-assets').mkdir()
        self.data = b'synthetic installer'
        self.digest = hashlib.sha256(self.data).hexdigest()
        self.normal = b'synthetic normal firmware'
        self.radio = b'synthetic radio firmware'
        self.manifest = dict(version='1.0.0', test_signing=False, source_revision='a' * 40,
                             installer=NAME, installer_sha256=self.digest, updater_public_key='public',
                             firmware=dict(bundle='touchdeck.tdfw', sha256=hashlib.sha256(self.normal).hexdigest(),
                                           manifest=dict(version='1.0.0')),
                             radio_firmware=dict(bundle='touchdeck-radio.tdfw', asset=RADIO,
                                                 sha256=hashlib.sha256(self.radio).hexdigest(),
                                                 manifest=dict(version='1.0.0', radio=True)))
        self.env = dict(os.environ, INSTALLER=NAME, EXPECTED_SHA256=self.digest,
                        SOURCE_REVISION='a' * 40, TAGGED_SHA='a' * 40, RELEASE_TAG='v1.0.0',
                        GITHUB_EVENT_NAME='push', GITHUB_REF_TYPE='tag', GITHUB_REF_NAME='v1.0.0',
                        GITHUB_ENV='github-env', RUNNER_TEMP=self.root.as_posix(),
                        UPDATER_PUBLIC_KEY='public', GH_REPO='example/project')

    def prepare(self):
        (self.assets / NAME).write_bytes(self.data)
        (self.assets / 'touchdeck.tdfw').write_bytes(self.normal)
        (self.assets / RADIO).write_bytes(self.radio)
        (self.assets / 'release-manifest.json').write_text(json.dumps(self.manifest))
        (self.assets / 'latest.json').write_text('{"notes":"untrusted build feed"}')
        (self.assets / (NAME + '.sig')).write_text('unsigned placeholder')
        (self.root / 'signed-assets' / (NAME + '.sig')).write_text('sign job signature')
        (self.assets / 'SHA256SUMS').write_text(''.join(
            f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n'
            for p in self.assets.iterdir() if p.name != 'SHA256SUMS'), newline='\n')

    def run_shell(self, name, prelude):
        script = self.root / 'check.sh'
        script.write_text(prelude + '\n' + workflow_shell(name), newline='\n')
        return subprocess.run(['bash', script.as_posix()], cwd=self.root, env=self.env,
                              capture_output=True, text=True)

    def test_signer_rejects_invalid_identity_before_signing(self):
        prelude = '''git() {
          if [ "$1" = fetch ]; then return 0; fi
          if [ "$1" = show ]; then printf '%s' '{"version":"1.0.0"}'; else return 1; fi
        }'''
        radio_in_installer = dict(self.manifest['firmware'], manifest=dict(version='1.0.0', radio=True))
        normal_as_radio = dict(self.manifest['radio_firmware'], manifest=dict(version='1.0.0'))
        for field, value in [(None, None), ('version', '9.9.9'), ('test_signing', True),
                             ('installer_sha256', 'b' * 64), ('source_revision', 'b' * 40),
                             ('firmware', radio_in_installer), ('radio_firmware', None),
                             ('radio_firmware', normal_as_radio)]:
            with self.subTest(field=field):
                original = dict(self.manifest)
                if field:
                    self.manifest[field] = value
                self.prepare()
                result = self.run_shell('Check build hash and version', prelude)
                self.assertEqual(result.returncode == 0, field is None, result.stderr)
                self.manifest = original
        # The firmware record and both bundles are checked before anything is signed.
        original = json.loads(json.dumps(self.manifest))
        cases = {
            'no firmware record': lambda: self.manifest.pop('firmware'),
            'a firmware record without its manifest': lambda: self.manifest['firmware'].pop('manifest'),
            'the installer bundle under another name': lambda: self.manifest['firmware'].update(bundle='other.tdfw'),
            'a normal bundle that is not the one built': lambda: setattr(self, 'normal', b'tampered normal'),
            'a radio bundle that is not the one built': lambda: setattr(self, 'radio', b'tampered radio'),
        }
        for case, change in cases.items():
            with self.subTest(case=case):
                self.manifest, self.normal, self.radio = json.loads(json.dumps(original)), b'synthetic normal firmware', b'synthetic radio firmware'
                change()
                for path in self.assets.iterdir():
                    path.unlink()
                self.prepare()
                result = self.run_shell('Check build hash and version', prelude)
                self.assertNotEqual(result.returncode, 0, case)
        self.manifest, self.normal, self.radio = original, b'synthetic normal firmware', b'synthetic radio firmware'
        for path in self.assets.iterdir():
            path.unlink()
        self.prepare()
        self.assertEqual(self.run_shell('Check build hash and version', prelude).returncode, 0)
        self.env['GITHUB_REF_NAME'] = 'v2.0.0'
        self.assertNotEqual(self.run_shell('Check build hash and version', prelude).returncode, 0)

    def test_publish_assembles_from_original_artifacts(self):
        self.prepare()
        annotation = json.dumps({'object': {'type': 'commit', 'sha': 'a' * 40},
                                 'message': 'Trusted tag notes', 'tagger': {'date': '2026-01-01T00:00:00Z'}})
        prelude = '''gh() {
          case "$2" in
            */git/ref/tags/v1.0.0) printf '%s' '{"object":{"type":"tag","sha":"annotation"}}' ;;
            */git/tags/annotation) printf '%s' 'ANNOTATION' ;;
            *) return 1 ;;
          esac
        }'''.replace('ANNOTATION', annotation)
        result = self.run_shell('Check the downloaded assets belong to this tag', prelude)
        self.assertEqual(result.returncode, 0, result.stderr)
        latest = json.loads((self.assets / 'latest.json').read_text())
        self.assertEqual(latest['notes'].strip(), 'Trusted tag notes')
        self.assertEqual(latest['platforms']['windows-x86_64']['signature'], 'sign job signature')
        self.assertEqual(latest['platforms']['windows-x86_64']['size'], len(self.data))
        self.prepare()
        (self.assets / NAME).write_bytes(b'tampered')
        self.assertNotEqual(self.run_shell('Check the downloaded assets belong to this tag', prelude).returncode, 0)
        self.data = b'tampered with consistent checksums'
        self.prepare()
        self.assertNotEqual(self.run_shell('Check the downloaded assets belong to this tag', prelude).returncode, 0)

    def test_publish_ships_the_radio_build_as_its_own_asset_and_never_as_the_installer_firmware(self):
        annotation = json.dumps({'object': {'type': 'commit', 'sha': 'a' * 40},
                                 'message': 'Trusted tag notes', 'tagger': {'date': '2026-01-01T00:00:00Z'}})
        prelude = '''gh() {
          case "$2" in
            */git/ref/tags/v1.0.0) printf '%s' '{"object":{"type":"tag","sha":"annotation"}}' ;;
            */git/tags/annotation) printf '%s' 'ANNOTATION' ;;
            *) return 1 ;;
          esac
        }'''.replace('ANNOTATION', annotation)
        step = 'Check the downloaded assets belong to this tag'
        self.prepare()
        self.assertEqual(self.run_shell(step, prelude).returncode, 0)
        sums = (self.assets / 'SHA256SUMS').read_text()
        summed = [line.split('  ', 1)[1] for line in sums.splitlines()]
        self.assertIn(RADIO, summed)
        self.assertIn('touchdeck.tdfw', summed)
        original = json.loads(json.dumps(self.manifest))
        cases = {
            'the installer firmware is the radio build': ('firmware', dict(original['firmware'], manifest=dict(radio=True))),
            'no firmware record': ('firmware', None),
            'a firmware record without its manifest': ('firmware', dict(original['firmware'], manifest=None)),
            'no radio build': ('radio_firmware', None),
            'the radio file is the normal build': ('radio_firmware', dict(original['radio_firmware'], manifest={})),
            'the radio file has another name': ('radio_firmware', dict(original['radio_firmware'], asset='touchdeck-radio.tdfw')),
            'the radio file is not the one built': ('radio_firmware', dict(original['radio_firmware'], sha256='b' * 64)),
        }
        for case, (field, value) in cases.items():
            with self.subTest(case=case):
                self.manifest = json.loads(json.dumps(original))
                self.manifest[field] = value
                for path in self.assets.iterdir():
                    path.unlink()
                self.prepare()
                self.assertNotEqual(self.run_shell(step, prelude).returncode, 0)
        self.manifest = original
        for path in self.assets.iterdir():
            path.unlink()
        self.prepare()
        (self.assets / RADIO).unlink()
        self.assertNotEqual(self.run_shell(step, prelude).returncode, 0)

    def test_draft_check_requires_the_radio_asset(self):
        self.prepare()
        for name, expect in [(None, 0), (RADIO, 1)]:
            with self.subTest(missing=name):
                if name:
                    (self.assets / name).unlink()
                # What the draft release lists: the same files and sizes as the build, as gh prints them.
                (self.root / 'listing.txt').write_text(''.join(
                    f'{p.name} {p.stat().st_size}\n' for p in sorted(self.assets.iterdir())), newline='\n')
                prelude = '''gh() {
                  if [ "$3" = "$RELEASE_TAG" ] && [ "$4" = --json ] && [ "$5" = assets ]; then cat "$RUNNER_TEMP/listing.txt"; fi
                }'''
                result = self.run_shell('Check draft assets', prelude)
                self.assertEqual(result.returncode != 0, bool(expect), result.stdout + result.stderr)

    @unittest.skipUnless(shutil.which('node'), 'node required')
    def test_sign_step_signs_with_the_release_secrets_and_version(self):
        self.prepare()
        shutil.copytree(ROOT / '.github/release-signer', self.root / '.github/release-signer')
        key = self.root / 'disposable-test-only.key'
        subprocess.run(['node', ROOT / '.github/release-signer/cli.mjs', 'generate', '--write-keys', key,
                        '--password', 'test-only'], check=True, capture_output=True)
        public = base64.b64decode(base64.b64decode(key.with_name(key.name + '.pub').read_text()).decode().split('\n')[1])
        self.env.update(RELEASE_VERSION='1.0.0', TAURI_SIGNING_PRIVATE_KEY=key.read_text(),
                        TAURI_SIGNING_PRIVATE_KEY_PASSWORD='test-only')
        key.unlink()
        result = self.run_shell('Sign the checked installer', '')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, '')
        lines = base64.b64decode((self.assets / (NAME + '.sig')).read_text()).decode().split('\n')
        self.assertEqual(base64.b64decode(lines[1])[:10], b'ED' + public[2:10])
        self.assertRegex(lines[2], rf'^trusted comment: timestamp:\d+\tfile:{NAME}\tversion:1\.0\.0$')
        self.env['TAURI_SIGNING_PRIVATE_KEY_PASSWORD'] = 'wrong'
        failed = self.run_shell('Sign the checked installer', '')
        self.assertNotEqual(failed.returncode, 0)
        self.assertEqual(failed.stderr.strip(), 'release-signer: Wrong password for the private key')


if __name__ == '__main__':
    unittest.main()