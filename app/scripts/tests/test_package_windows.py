import argparse
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location("package_windows", Path(__file__).parents[1] / "package_windows.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class ProductionCompanionTests(unittest.TestCase):
    def test_plain_companion_is_accepted(self):
        package.verify_production_companion(b"MZ/api/v1/system/shutdown")

    def test_test_hook_routes_or_arguments_are_rejected(self):
        for marker in (b"/test/pairing-delivery", b"/test/devices/", b"--emit-ts", b"--test-real-device-console"):
            with self.subTest(marker=marker), self.assertRaisesRegex(ValueError, "Test hooks"):
                package.verify_production_companion(b"MZ" + marker)

    def test_non_executable_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Windows executable"):
            package.verify_production_companion(b"ELF")

    def test_lifecycle_update_is_the_next_patch(self):
        self.assertEqual(package.next_patch("0.2.9"), "0.2.10")

    def test_captured_output_is_utf8_on_any_code_page(self):
        script = "import sys; sys.stdout.buffer.write('\\u201dOpen TouchDeck\\u201d'.encode('utf-8'))"
        self.assertEqual(package.capture([sys.executable, "-c", script]), "”Open TouchDeck”")


FIXTURES = Path(__file__).resolve().parents[2] / "crates/td-link/tests/fixtures"
NORMAL = FIXTURES / "touchdeck-test.tdfw"
RADIO = FIXTURES / "touchdeck-test-radio.tdfw"


class PrebuiltFirmwareTests(unittest.TestCase):
    """Test-signed candidates may reuse built bundles, but each build only through its own option."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.out = Path(self.temp.name)
        (self.out / "payload").mkdir()

    def test_the_normal_bundle_alone_carries_no_radio_build(self):
        normal, radio = package.prebuilt_firmware(self.out, "a" * 40, NORMAL)
        self.assertEqual(normal["bundle"], "touchdeck.tdfw")
        self.assertNotIn("radio", normal["manifest"])
        self.assertIsNone(radio)

    def test_both_bundles_land_apart_and_each_is_its_own_build(self):
        normal, radio = package.prebuilt_firmware(self.out, "a" * 40, NORMAL, RADIO)
        self.assertEqual((normal["bundle"], radio["bundle"]), ("touchdeck.tdfw", "touchdeck-radio.tdfw"))
        # Only the normal build is installed as a file; the radio build stays out of the payload.
        self.assertEqual(sorted(p.name for p in (self.out / "payload/firmware").iterdir()), ["touchdeck.tdfw"])
        self.assertTrue((self.out / package.RADIO_FOLDER / "touchdeck-radio.tdfw").is_file())
        self.assertTrue(radio["manifest"]["radio"])
        self.assertEqual(radio["sha256"], package.sha(RADIO.read_bytes()))
        self.assertEqual(package.radio_asset("1.2.3"), "touchdeck-radio-1.2.3.tdfw")

    def test_a_radio_bundle_is_never_the_normal_firmware(self):
        with self.assertRaisesRegex(ValueError, "--firmware-bundle holds the radio build"):
            package.prebuilt_firmware(self.out, "a" * 40, RADIO)

    def test_a_normal_bundle_is_never_the_radio_firmware(self):
        with self.assertRaisesRegex(ValueError, "--radio-firmware-bundle holds the normal build"):
            package.prebuilt_firmware(self.out, "a" * 40, NORMAL, NORMAL)

    def test_a_packed_bundle_must_be_the_build_its_name_says(self):
        self.assertTrue(package.checked_bundle(RADIO, radio=True)["radio"])
        with self.assertRaisesRegex(ValueError, "is the radio build, not the normal build"):
            package.checked_bundle(RADIO, radio=False)
        with self.assertRaisesRegex(ValueError, "is the normal build, not the radio build"):
            package.checked_bundle(NORMAL, radio=True)

    def test_the_radio_bundle_option_needs_the_normal_one(self):
        args = argparse.Namespace(test_signing=True, public_key_file=None, firmware_bundle=None,
                                  radio_firmware_bundle=RADIO)
        with mock.patch.object(package.sys, "platform", "win32"), mock.patch.dict(package.os.environ, clear=False):
            for key in [key for key in package.os.environ if key.upper().startswith("TAURI_SIGNING_PRIVATE_KEY")]:
                del package.os.environ[key]
            with self.assertRaisesRegex(ValueError, "--radio-firmware-bundle needs --firmware-bundle"):
                package.build(args)


if __name__ == "__main__":
    unittest.main()
