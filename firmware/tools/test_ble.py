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
    if test == "tile_bluetooth_test.c":
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

    def test_keyboard_sync_requires_private_address(self):
        # Compile the production sync callback against the SDK boundary only.
        source = (BLE / "td_ble_nimble.c").read_text()
        callback = source[source.index("static void on_sync(void)"):source.index("static void on_reset(")]
        harness = r"""
#include <assert.h>
#include <stdint.h>
#define ESP_LOGE(...) ((void)0)
#define ESP_LOGI(...) ((void)0)
#define BLE_OWN_ADDR_RPA_PUBLIC_DEFAULT 2
#define BLE_OWN_ADDR_RPA_RANDOM_DEFAULT 3
static struct { uint8_t own_addr_type; int core; } state, *s = &state;
static int failure, synced;
static int ble_hs_util_ensure_addr(int random) { (void)random; return failure == 1; }
static int ble_hs_id_infer_auto(int privacy, uint8_t *out) {
    *out = privacy ? BLE_OWN_ADDR_RPA_PUBLIC_DEFAULT : 0;
    return failure == 2;
}
static void signal_gatt_version(void) {}
static void lock(void) {}
static void unlock(void) {}
static void td_ble_core_synced(int *core) { (void)core; synced++; }
""" + callback + r"""
int main(void) {
    on_sync();
    assert(synced == 1);
    assert(s->own_addr_type == BLE_OWN_ADDR_RPA_PUBLIC_DEFAULT);
    for (failure = 1; failure <= 2; failure++) {
        synced = 0;
        on_sync();
        assert(synced == 0);
    }
}
"""
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            out = Path(tmp)
            (out / "sync.c").write_text(harness)
            exe = out / ("sync.exe" if os.name == "nt" else "sync")
            if os.name == "nt":
                command = ["cl", "/nologo", "/std:clatest", "/W3", "/WX",
                           str(out / "sync.c"), "/Fe:" + str(exe)]
            else:
                command = [os.environ.get("CC", "cc"), "-std=c11", "-Wall", "-Wextra", "-Werror",
                           str(out / "sync.c"), "-o", str(exe)]
            subprocess.run(command, cwd=out, check=True)
            subprocess.run([str(exe)], check=True)

    def test_gap_name_is_generic_before_authentication(self):
        source = (BLE / "td_ble_nimble.c").read_text()
        start = source.index("    if (td_device_name_get(s->name)")
        end = source.index("    ble_svc_gap_device_appearance_set", start)
        harness = r"""
#define _POSIX_C_SOURCE 200809L
#include <assert.h>
#include <string.h>
#define ESP_OK 0
#define MYNEWT_VAL(name) 31
static struct { char name[64]; } state, *s = &state;
static int td_device_name_get(char *name) { strcpy(name, "Private chosen Device name"); return 0; }
static void ble_svc_gap_device_name_set(const char *name) { assert(strcmp(name, "Open TouchDeck") == 0); }
int main(void) {
""" + source[start:end] + "}"
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            out = Path(tmp)
            (out / "name.c").write_text(harness)
            exe = out / ("name.exe" if os.name == "nt" else "name")
            if os.name == "nt":
                command = ["cl", "/nologo", "/std:clatest", "/W3", "/WX", "/D_CRT_SECURE_NO_WARNINGS",
                           str(out / "name.c"), "/Fe:" + str(exe)]
            else:
                command = [os.environ.get("CC", "cc"), "-std=c11", "-Wall", "-Wextra", "-Werror",
                           str(out / "name.c"), "-o", str(exe)]
            subprocess.run(command, cwd=out, check=True)
            subprocess.run([str(exe)], check=True)

    def test_tile_destinations(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            build_and_run("tile_bluetooth_test.c", ["td_ble_hid.c"], Path(tmp))

    def test_report_map_and_gatt_table(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            build_and_run("ble_hid_test.c", ["td_ble_hid.c"], Path(tmp))

    def test_core_against_a_fake_radio(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            build_and_run("ble_core_test.c", ["td_ble_core.c", "td_ble_hid.c", "td_ble_msgs.c"], Path(tmp))


if __name__ == "__main__":
    unittest.main()
