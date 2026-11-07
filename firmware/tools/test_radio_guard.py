"""The radio guards: the normal build fails on any radio option or component, and the radio build passes only with
dormant BLE and an unjoined station. The release recipe's audit must reject the same builds.

    python -B firmware/tools/test_radio_guard.py

TD_GUARD_NORMAL_BUILD and TD_GUARD_RADIO_BUILD, when set, name real build directories, and the test also checks
them. CI sets each in the job that built it.
"""

import importlib.util
import json
import os
from pathlib import Path
import re
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


guard = load("radio_guard", ROOT / "firmware/tools/radio_guard.py")
package = load("package_windows", ROOT / "app/scripts/package_windows.py")

NORMAL_SETTINGS = {"IDF_TARGET": "esp32s3", "ESP_CONSOLE_USB_SERIAL_JTAG": True}
NORMAL_COMPONENTS = ["main", "td_link", "td_console", "esp_system", "nvs_flash", "mbedtls", "esp_psram"]
RADIO_SETTINGS = {**NORMAL_SETTINGS, **guard.RADIO_BUILD_OPTIONS, "TD_RADIO_INTERNAL_FLOOR": 24576}
RADIO_COMPONENTS = NORMAL_COMPONENTS + ["td_radio", "td_netlink", "td_netlink_core", "esp_wifi", "esp_netif", "lwip",
                                        "bt", "esp_phy", "esp_coex", "wpa_supplicant"]
RADIO_SYMBOLS = {"app_main", "td_radio_start", "esp_wifi_init", "esp_wifi_start", "td_netlink_start",
                 "ble_gatts_count_cfg"}
# Spelled out here, not read from the guard, so dropping a name from the guard fails a test.
FORBIDDEN_OPTIONS = ("ESP_WIFI_ENABLED", "ESP_HOST_WIFI_ENABLED", "BT_ENABLED", "BT_CONTROLLER_ENABLED",
                     "BT_NIMBLE_ENABLED", "BT_BLUEDROID_ENABLED", "TD_RADIO_BUILD", "TD_QEMU_TEST", "ETH_USE_OPENETH")
FORBIDDEN_COMPONENTS = ("bt", "esp_wifi", "esp_phy", "esp_coex", "wpa_supplicant", "esp_netif", "esp_netif_stack",
                        "lwip", "td_net",
                        "td_ble", "td_radio", "td_netlink", "td_netlink_core", "td_netlink_v2", "mdns",
                        "espressif__mdns")
DORMANT = ("esp_bt_controller_init", "esp_bt_controller_enable", "nimble_port_init", "nimble_port_freertos_init",
           "ble_hs_start", "ble_gap_adv_start", "ble_gap_ext_adv_start", "ble_gap_disc", "ble_gap_connect",
           "esp_wifi_connect", "esp_wifi_scan_start", "esp_wifi_wps_start", "esp_wifi_80211_tx",
           "esp_smartconfig_start", "esp_now_init")


class BuildDirs:
    def __init__(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.count = 0

    def make(self, settings, components):
        self.count += 1
        build = Path(self.tmp.name) / f"build-{self.count}"
        (build / "config").mkdir(parents=True)
        (build / "config/sdkconfig.json").write_text(json.dumps(settings), encoding="utf-8")
        description = {"build_components": components, "app_elf": "touchdeck.elf", "c_compiler": ""}
        (build / "project_description.json").write_text(json.dumps(description), encoding="utf-8")
        return build


class NormalBuildTests(unittest.TestCase):
    def setUp(self):
        self.dirs = BuildDirs()
        self.addCleanup(self.dirs.tmp.cleanup)

    def audits(self):
        """The CI guard and the release recipe's audit, which must agree."""
        return (("radio_guard", guard.check_normal), ("package_windows", package.audit_radio))

    def test_radio_free_build_passes(self):
        build = self.dirs.make(NORMAL_SETTINGS, NORMAL_COMPONENTS)
        for name, audit in self.audits():
            with self.subTest(audit=name):
                self.assertTrue(audit(build)["usb_serial_jtag"])

    def test_every_radio_option_fails(self):
        for key in FORBIDDEN_OPTIONS:
            build = self.dirs.make({**NORMAL_SETTINGS, key: True}, NORMAL_COMPONENTS)
            for name, audit in self.audits():
                with self.subTest(option=key, audit=name), self.assertRaisesRegex(ValueError, f"Radio enabled: {key}"):
                    audit(build)

    def test_every_radio_component_fails(self):
        for component in FORBIDDEN_COMPONENTS:
            build = self.dirs.make(NORMAL_SETTINGS, NORMAL_COMPONENTS + [component])
            for name, audit in self.audits():
                with self.subTest(component=component, audit=name), \
                        self.assertRaisesRegex(ValueError, f"Radio components linked: .*'{component}'"):
                    audit(build)

    def test_similar_names_pass(self):
        build = self.dirs.make(NORMAL_SETTINGS, NORMAL_COMPONENTS + ["espressif__mdns_extras_doc", "td_network_ui"])
        self.assertEqual(guard.check_normal(build)["radio_components"], [])

    def test_the_radio_build_fails_the_normal_guard(self):
        build = self.dirs.make(RADIO_SETTINGS, RADIO_COMPONENTS)
        for name, audit in self.audits():
            with self.subTest(audit=name), self.assertRaisesRegex(ValueError, "Radio enabled"):
                audit(build)

    def test_lost_console_or_target_fails(self):
        for settings, message in (({**NORMAL_SETTINGS, "ESP_CONSOLE_USB_SERIAL_JTAG": False}, "USB-Serial/JTAG"),
                                  ({**NORMAL_SETTINGS, "IDF_TARGET": "esp32"}, "IDF_TARGET")):
            build = self.dirs.make(settings, NORMAL_COMPONENTS)
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                guard.check_normal(build)


class RadioBuildTests(unittest.TestCase):
    def setUp(self):
        self.dirs = BuildDirs()
        self.addCleanup(self.dirs.tmp.cleanup)

    def test_radio_build_passes(self):
        build = self.dirs.make(RADIO_SETTINGS, RADIO_COMPONENTS)
        result = guard.check_radio(build, symbols=RADIO_SYMBOLS)
        self.assertIn("td_radio", result["radio_components"])
        self.assertIn("nimble_port_init", result["dormant"])

    def test_every_required_option_is_checked(self):
        for key, want in guard.RADIO_BUILD_OPTIONS.items():
            build = self.dirs.make({**RADIO_SETTINGS, key: not want}, RADIO_COMPONENTS)
            with self.subTest(option=key), self.assertRaisesRegex(ValueError, f"needs {key}="):
                guard.check_radio(build, symbols=RADIO_SYMBOLS)

    def test_missing_option_fails(self):
        for key in list(guard.RADIO_BUILD_OPTIONS) + ["TD_RADIO_INTERNAL_FLOOR"]:
            settings = {name: value for name, value in RADIO_SETTINGS.items() if name != key}
            build = self.dirs.make(settings, RADIO_COMPONENTS)
            with self.subTest(option=key), self.assertRaisesRegex(ValueError, f"needs {key}"):
                guard.check_radio(build, symbols=RADIO_SYMBOLS)

    def test_floor_below_24_kb_fails(self):
        build = self.dirs.make({**RADIO_SETTINGS, "TD_RADIO_INTERNAL_FLOOR": 16384}, RADIO_COMPONENTS)
        with self.assertRaisesRegex(ValueError, "TD_RADIO_INTERNAL_FLOOR >= 24576"):
            guard.check_radio(build, symbols=RADIO_SYMBOLS)

    def test_missing_radio_component_fails(self):
        for component in guard.RADIO_BUILD_COMPONENTS:
            components = [name for name in RADIO_COMPONENTS if name != component]
            build = self.dirs.make(RADIO_SETTINGS, components)
            with self.subTest(component=component), self.assertRaisesRegex(ValueError, "missing components"):
                guard.check_radio(build, symbols=RADIO_SYMBOLS)

    def test_any_ble_start_advertise_scan_or_wifi_join_fails(self):
        build = self.dirs.make(RADIO_SETTINGS, RADIO_COMPONENTS)
        for symbol in DORMANT:
            with self.subTest(symbol=symbol), self.assertRaisesRegex(ValueError, f"must stay dormant: \\['{symbol}'\\]"):
                guard.check_radio(build, symbols=RADIO_SYMBOLS | {symbol})

    def test_unwired_station_fails(self):
        build = self.dirs.make(RADIO_SETTINGS, RADIO_COMPONENTS)
        with self.assertRaisesRegex(ValueError, "does not link: \\['esp_wifi_start'\\]"):
            guard.check_radio(build, symbols=RADIO_SYMBOLS - {"esp_wifi_start"})

    def test_unwired_network_service_fails(self):
        build = self.dirs.make(RADIO_SETTINGS, RADIO_COMPONENTS)
        with self.assertRaisesRegex(ValueError, "does not link: \\['td_netlink_start'\\]"):
            guard.check_radio(build, symbols=RADIO_SYMBOLS - {"td_netlink_start"})

    def test_qemu_test_image_fails(self):
        """The QEMU test image opens a pairing window by itself, so no real radio build may carry any of it."""
        for key in ("TD_QEMU_TEST", "ETH_USE_OPENETH"):
            build = self.dirs.make({**RADIO_SETTINGS, key: True}, RADIO_COMPONENTS)
            with self.subTest(option=key), self.assertRaisesRegex(ValueError, f"test-only options: \\['{key}'\\]"):
                guard.check_radio(build, symbols=RADIO_SYMBOLS)
        build = self.dirs.make(RADIO_SETTINGS, RADIO_COMPONENTS)
        for symbol in ("td_radio_qemu_eth_start", "esp_eth_mac_new_openeth"):
            with self.subTest(symbol=symbol), self.assertRaisesRegex(ValueError, f"test-only code: \\['{symbol}'\\]"):
                guard.check_radio(build, symbols=RADIO_SYMBOLS | {symbol})

    def test_no_firmware_source_calls_a_dormant_function(self):
        """The same rule at the source, so a normal-build CI run catches it before any radio build links it."""
        sources = [*(ROOT / "firmware/components").rglob("*.[ch]"), *(ROOT / "firmware/main").rglob("*.[ch]")]
        self.assertTrue(any(path.name == "td_radio.c" for path in sources))
        call = re.compile(r"\b(" + "|".join(guard.DORMANT_SYMBOLS) + r")\s*\(")
        found = [f"{path.relative_to(ROOT).as_posix()}: {match.group(1)}" for path in sources
                 for match in call.finditer(path.read_text(encoding="utf-8", errors="replace"))]
        self.assertEqual(found, [])

    def test_static_internal_ram_ceiling(self):
        report = Path(self.dirs.tmp.name) / "size.json"
        for used, ok in ((151083, True), (153600, True), (153601, False)):
            report.write_text(json.dumps({"layout": [{"name": "Flash Code", "used": 9}, {"name": "DIRAM", "used": used}]}),
                              encoding="utf-8")
            with self.subTest(used=used):
                if ok:
                    self.assertEqual(guard.check_diram(report), used)
                else:
                    with self.assertRaisesRegex(ValueError, "over the 153600-byte ceiling"):
                        guard.check_diram(report)

    def test_lost_console_fails(self):
        build = self.dirs.make({**RADIO_SETTINGS, "ESP_CONSOLE_USB_SERIAL_JTAG": False}, RADIO_COMPONENTS)
        with self.assertRaisesRegex(ValueError, "USB-Serial/JTAG"):
            guard.check_radio(build, symbols=RADIO_SYMBOLS)


bundle_tool = load("make_bundle", ROOT / "firmware/tools/make_bundle.py")


def fake_image():
    """The smallest touchdeck.bin make_bundle reads: an image header and an app descriptor for touchdeck."""
    image = bytearray(0x200)
    image[0] = 0xE9
    image[0x20:0x24] = (0xABCD5432).to_bytes(4, "little")
    image[0x20 + 16:0x20 + 22] = b"v1.0.0"
    image[0x20 + 48:0x20 + 57] = b"touchdeck"
    return bytes(image)


class BundleMarkTests(unittest.TestCase):
    """A radio bundle says so in its manifest, and the installer recipe refuses it as --firmware-bundle."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def bundle(self, name, radio):
        build = Path(self.tmp.name) / name
        for relative, data in (("bootloader/bootloader.bin", b"boot"), ("partition_table/partition-table.bin", b"pt"),
                               ("touchdeck.bin", fake_image())):
            (build / relative).parent.mkdir(parents=True, exist_ok=True)
            (build / relative).write_bytes(data)
        flash = {"flash_files": {"0x0": "bootloader/bootloader.bin", "0x8000": "partition_table/partition-table.bin",
                                 "0x40000": "touchdeck.bin"}}
        (build / "flasher_args.json").write_text(json.dumps(flash), encoding="utf-8")
        (build / "config").mkdir()
        settings = {**NORMAL_SETTINGS, **({"TD_RADIO_BUILD": True} if radio else {})}
        (build / "config/sdkconfig.json").write_text(json.dumps(settings), encoding="utf-8")
        path, manifest = bundle_tool.make_bundle(build, build / "out")
        bundle_tool.check_bundle(path)
        return path, manifest

    def test_normal_bundle_has_no_mark_and_is_accepted(self):
        path, manifest = self.bundle("normal", radio=False)
        self.assertNotIn("radio", manifest)
        out = Path(self.tmp.name) / "package-normal"
        (out / "payload").mkdir(parents=True)
        self.assertEqual(package.prebuilt_firmware(out, "rev", path)["manifest"], manifest)

    def test_radio_bundle_is_marked_and_refused(self):
        path, manifest = self.bundle("radio", radio=True)
        self.assertIs(manifest["radio"], True)
        out = Path(self.tmp.name) / "package-radio"
        (out / "payload").mkdir(parents=True)
        with self.assertRaisesRegex(ValueError, "radio build"):
            package.prebuilt_firmware(out, "rev", path)


@unittest.skipUnless(os.environ.get("TD_GUARD_NORMAL_BUILD"), "set TD_GUARD_NORMAL_BUILD to check a real normal build")
class RealNormalBuildTests(unittest.TestCase):
    """A real normal build, linked by ESP-IDF, passes the normal guard and the release audit and fails the radio one."""

    build = Path(os.environ.get("TD_GUARD_NORMAL_BUILD", "."))

    def test_normal_build_is_radio_free(self):
        guard.check_normal(self.build)
        package.audit_radio(self.build)

    def test_normal_build_fails_the_radio_guard(self):
        with self.assertRaisesRegex(ValueError, "Radio build needs"):
            guard.check_radio(self.build)


@unittest.skipUnless(os.environ.get("TD_GUARD_RADIO_BUILD"), "set TD_GUARD_RADIO_BUILD to check a real radio build")
class RealRadioBuildTests(unittest.TestCase):
    """A real radio build passes its own guard, image check included, and fails the normal guard and the release
    audit, so it can never ship as a release."""

    build = Path(os.environ.get("TD_GUARD_RADIO_BUILD", "."))

    def test_radio_build_passes_its_own_guard(self):
        guard.check_radio(self.build)

    def test_radio_build_fails_the_normal_guard_and_the_release_audit(self):
        for audit in (guard.check_normal, package.audit_radio):
            with self.subTest(audit=audit.__name__), self.assertRaisesRegex(ValueError, "Radio enabled"):
                audit(self.build)


if __name__ == "__main__":
    unittest.main()
