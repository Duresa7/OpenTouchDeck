import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BLE = ROOT / "components/td_ble"
HOST = ROOT / "tools/host"
CJSON = ROOT / "managed_components/espressif__cjson/cJSON"


def build_and_run(test, sources, out):
    """Compiles the radio-free td_ble sources with a host test and runs it. Warnings fail the build."""
    includes = [BLE, ROOT / "components/keys/include", ROOT / "components/td_ui_core/include", CJSON]
    exe = out / ("test.exe" if os.name == "nt" else "test")
    files = [HOST / test] + [BLE / s for s in sources]
    if test == "tile_hid_test.c":
        files += [ROOT / "components/td_ui_core/ui_pages.c"]
    if any(s.startswith("td_ble_msgs") for s in sources):
        files.append(CJSON / "cJSON.c")
    if os.name == "nt":
        command = ["cl", "/nologo", "/std:clatest", "/W3", "/WX", "/D_CRT_SECURE_NO_WARNINGS"]
        command += ["/I" + str(p) for p in includes] + [str(p) for p in files] + ["/Fe:" + str(exe)]
    else:
        command = [os.environ.get("CC", "cc"), "-std=c11", "-Wall", "-Wextra", "-Werror"]
        command += ["-I" + str(p) for p in includes] + [str(p) for p in files] + ["-lm", "-o", str(exe)]
    subprocess.run(command, cwd=out, check=True)
    subprocess.run([str(exe)], cwd=out, check=True)


class BleTest(unittest.TestCase):
    """The radio build's Bluetooth keyboard: tools/host/ble_hid_test.c and tools/host/ble_core_test.c."""

    def test_tile_destinations(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            build_and_run("tile_hid_test.c", ["td_ble_hid.c"], Path(tmp))

    def test_report_map_and_gatt_table(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            build_and_run("ble_hid_test.c", ["td_ble_hid.c"], Path(tmp))

    def test_core_against_a_fake_radio(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            build_and_run("ble_core_test.c", ["td_ble_core.c", "td_ble_hid.c", "td_ble_msgs.c"], Path(tmp))


if __name__ == "__main__":
    unittest.main()
