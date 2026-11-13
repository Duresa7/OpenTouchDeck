"""Tests for release_version.py against temporary copies of the real version files.

    python -m unittest discover -s app/scripts/tests -p "test_release_version.py"
"""

import contextlib
import importlib.util
import io
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("release_version", ROOT / "app/scripts/release_version.py")
release_version = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_version)
FILES = tuple(release_version.PATTERNS)
LOCK = release_version.LOCK


class ReleaseVersionTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="touchdeck-release-version-")
        self.root = Path(self.temp.name)
        for relative in (*FILES, LOCK):
            (self.root / relative).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, self.root / relative)
        self.version = release_version.read_versions(self.root)[FILES[0]]

    def tearDown(self):
        self.temp.cleanup()

    def run_cli(self, *args):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()) as errors:
            code = release_version.main(["--root", str(self.root), *args])
        return code, errors.getvalue()

    def next_version(self):
        major, minor, patch = (int(part) for part in self.version.split("."))
        return f"{major}.{minor}.{patch + 1}"

    def test_check_passes_on_repo_copy(self):
        self.assertEqual(self.run_cli("check")[0], 0)

    def test_check_tag_must_be_v_plus_version(self):
        self.assertEqual(self.run_cli("check", "--tag", f"v{self.version}")[0], 0)
        for tag in (self.version, f"v{self.next_version()}", f"V{self.version}", f"v{self.version}-rc1"):
            code, errors = self.run_cli("check", "--tag", tag)
            self.assertEqual(code, 1, tag)
            self.assertIn("Tag", errors)

    def test_check_fails_when_any_file_disagrees(self):
        for relative in FILES:
            with self.subTest(relative=relative):
                path = self.root / relative
                original = path.read_bytes()
                pattern = release_version.PATTERNS[relative]
                path.write_bytes(pattern.sub(lambda m: m.group(1) + "9.9.9" + m.group(3), original.decode("utf-8"),
                                             count=1).encode("utf-8"))
                code, errors = self.run_cli("check")
                path.write_bytes(original)
                self.assertEqual(code, 1)
                self.assertIn("disagree", errors)

    def test_check_fails_when_a_lock_entry_disagrees(self):
        path = self.root / LOCK
        original = path.read_bytes()
        for crate in release_version.CRATES:
            with self.subTest(crate=crate):
                pattern = release_version.lock_pattern(crate)
                path.write_bytes(pattern.sub(lambda m: m.group(1) + "9.9.9" + m.group(3), original.decode("utf-8"),
                                             count=1).encode("utf-8"))
                code, errors = self.run_cli("check")
                path.write_bytes(original)
                self.assertEqual(code, 1)
                self.assertIn("disagree", errors)

    def test_bump_leaves_dependency_versions_alone(self):
        relative = "app/crates/td-core/Cargo.toml"
        path = self.root / relative
        text = path.read_bytes().decode("utf-8")
        # Tables of their own, so the crate's own [dependencies] or [dev-dependencies] can't clash.
        text += (
            "\n[target.'cfg(any())'.dev-dependencies]\nsome-dep = { version = \"0.1.0\" }\n"
            '\n[target.\'cfg(any())\'.dependencies.other]\nversion = "2.0.0"\n'
        )
        path.write_bytes(text.encode("utf-8"))
        self.assertEqual(self.run_cli("bump", "7.8.9")[0], 0)
        updated = path.read_bytes().decode("utf-8")
        self.assertIn('some-dep = { version = "0.1.0" }', updated)
        self.assertIn('[target.\'cfg(any())\'.dependencies.other]\nversion = "2.0.0"', updated)
        self.assertEqual(updated.count('version = "7.8.9"'), 1)

    def test_bump_updates_lock_entries_only_for_workspace_crates(self):
        lock = (self.root / LOCK).read_bytes().decode("utf-8")
        other_versions = lock.count('version = "') - len(release_version.CRATES)
        self.assertEqual(self.run_cli("bump", "7.8.9")[0], 0)
        updated = (self.root / LOCK).read_bytes().decode("utf-8")
        self.assertEqual(updated.count('version = "7.8.9"'), len(release_version.CRATES))
        self.assertEqual(updated.count('version = "') - len(release_version.CRATES), other_versions)

    def test_bump_changes_only_version_strings(self):
        before = {relative: (self.root / relative).read_bytes() for relative in FILES}
        new = self.next_version()
        self.assertEqual(self.run_cli("bump", new)[0], 0)
        self.assertEqual(self.run_cli("check", "--tag", f"v{new}")[0], 0)
        for relative in FILES:
            after = (self.root / relative).read_bytes()
            self.assertNotEqual(after, before[relative], relative)
            # Putting the old version back must restore every byte, line endings included.
            pattern = release_version.PATTERNS[relative]
            restored = pattern.sub(lambda m: m.group(1) + self.version + m.group(3), after.decode("utf-8"), count=1)
            self.assertEqual(restored.encode("utf-8"), before[relative], relative)
            self.assertEqual(after.count(b"\r"), before[relative].count(b"\r"), relative)

    def test_bump_handles_crlf_checkouts(self):
        for relative in FILES:
            path = self.root / relative
            path.write_bytes(path.read_bytes().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n"))
        self.assertEqual(self.run_cli("bump", "1.2.3")[0], 0)
        self.assertEqual(self.run_cli("check", "--tag", "v1.2.3")[0], 0)
        for relative in FILES:
            data = (self.root / relative).read_bytes()
            self.assertEqual(data.count(b"\r"), data.count(b"\r\n"), relative)

    def test_bump_rejects_bad_versions_without_writing(self):
        before = {relative: (self.root / relative).read_bytes() for relative in FILES}
        for version in ("1.2", "v1.2.3", "1.2.3-rc1", "1.2.3.4", ""):
            code, errors = self.run_cli("bump", version)
            self.assertEqual(code, 1, version)
            self.assertIn("not X.Y.Z", errors)
        for relative in FILES:
            self.assertEqual((self.root / relative).read_bytes(), before[relative], relative)


if __name__ == "__main__":
    unittest.main()
