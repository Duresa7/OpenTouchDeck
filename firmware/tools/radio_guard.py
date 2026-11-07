#!/usr/bin/env python3
"""Checks a firmware build directory against the rules for its kind: the normal build or the radio build.

The normal build is what every release ships. It must stay radio-free: no Wi-Fi or Bluetooth option on, and no radio
or network component linked. The release recipe (app/scripts/package_windows.py) runs the same check before it packs
the firmware, so a mistake here cannot ship radios to everyone.

The radio build is the opt-in build that `idf.py -DTD_RADIO_BUILD=1` makes (see docs/firmware.md). It must have Wi-Fi,
lwIP and NimBLE, keep Wi-Fi and lwIP buffers in PSRAM and the BLE controller in flash, and keep BLE a peripheral that
pairs only with LE Secure Connections. Only the Bluetooth keyboard (td_ble) may start BLE or advertise, and it does so
only when the person's stored setting is on: the host tests in firmware/tools/test_ble.py check that, and
test_radio_guard.py checks no other source calls those functions. Nothing in the linked image may scan, start a
connection or use extended advertising. The Wi-Fi station may join the one network set over USB (the `wifi.*` link
messages), and only the station's driver code (WIFI_JOIN_OWNER) may call the join, but it never scans. It runs
net-v2's TCP service, and nothing from the QEMU test image (-DTD_QEMU_TEST=1), which opens a pairing window by
itself, may be in it.

    python firmware/tools/radio_guard.py normal <build dir>
    python firmware/tools/radio_guard.py radio <build dir> [--size-json <idf.py size --format json2 output>]
"""

import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

TARGET = "esp32s3"

# Options that burn an eFuse on the Device or start encrypting its flash. Burning is irreversible, so no build, normal or
# radio, may turn one on until the operator decides (docs/adr/0010-nvs-stays-unencrypted-until-the-operator-decides.md).
# NVS_ENCRYPTION is on the list because on the ESP32-S3 the first nvs_flash_init then burns an HMAC key into an eFuse
# block.
IRREVERSIBLE_OPTIONS = (
    "NVS_ENCRYPTION",
    "SECURE_FLASH_ENC_ENABLED",
    "SECURE_BOOT",
    "SECURE_BOOT_V2_ENABLED",
    "BOOTLOADER_APP_ANTI_ROLLBACK",
)

# Options that turn a radio or the radio build on. Any one of them fails the normal build.
RADIO_OPTIONS = (
    "ESP_WIFI_ENABLED",
    "ESP_HOST_WIFI_ENABLED",
    "BT_ENABLED",
    "BT_CONTROLLER_ENABLED",
    "BT_NIMBLE_ENABLED",
    "BT_BLUEDROID_ENABLED",
    "TD_RADIO_BUILD",
    "TD_QEMU_TEST",
    "ETH_USE_OPENETH",
)

# Components that only a radio build may link: the radio drivers, the network stack and our own radio components.
RADIO_COMPONENTS = frozenset({
    "bt",
    "esp_coex",
    "esp_netif",
    "esp_netif_stack",
    "esp_phy",
    "esp_wifi",
    "lwip",
    "td_ble",
    "td_net",
    "td_netlink",
    "td_netlink_core",
    "td_radio",
    "wpa_supplicant",
})
# mDNS arrives from the component registry as espressif__mdns, and the M3 network link was td_netlink and
# td_netlink_core. Any later name in those families counts too.
RADIO_COMPONENT_PATTERN = re.compile(r"(^|__)mdns$|netlink")

# The radio build's required options, and the value each must have.
RADIO_BUILD_OPTIONS = {
    "TD_RADIO_BUILD": True,
    "ESP_WIFI_ENABLED": True,
    "BT_ENABLED": True,
    "BT_CONTROLLER_ENABLED": True,
    "BT_NIMBLE_ENABLED": True,
    "BT_BLUEDROID_ENABLED": False,
    # Wi-Fi and lwIP buffers come from PSRAM, NimBLE's own memory too, and the BLE controller runs from flash.
    "SPIRAM_TRY_ALLOCATE_WIFI_LWIP": True,
    "BT_NIMBLE_MEM_ALLOC_MODE_EXTERNAL": True,
    # Static .bss of Wi-Fi, lwIP, wpa_supplicant and the link's frame buffers lives in PSRAM, which pays for BLE's
    # internal RAM.
    "SPIRAM_ALLOW_BSS_SEG_EXTERNAL_MEMORY": True,
    "BT_CTRL_RUN_IN_FLASH_ONLY": True,
    # No scanning or connecting role is even compiled in, and no extended advertising.
    "BT_NIMBLE_ROLE_OBSERVER": False,
    "BT_NIMBLE_ROLE_CENTRAL": False,
    "BT_NIMBLE_ROLE_PERIPHERAL": True,
    "BT_NIMBLE_EXT_ADV": False,
    # The Bluetooth keyboard (#276): LE Secure Connections only, bonds kept in NVS, one host at a time, at most four
    # bonds, and Peripheral Preferred Connection Parameters that match its one request: 7.5-50 ms, latency 0, 4 s.
    "BT_NIMBLE_SM_LEGACY": False,
    "BT_NIMBLE_SM_SC": True,
    "BT_NIMBLE_SM_SC_ONLY": 1,
    # The controller resolves bonded hosts' private addresses (the resolving list td_ble writes with the accept list).
    "BT_NIMBLE_HS_PVCY": True,
    "BT_NIMBLE_NVS_PERSIST": True,
    "BT_NIMBLE_MAX_CONNECTIONS": 1,
    "BT_NIMBLE_MAX_BONDS": 4,
    "BT_NIMBLE_SVC_GAP_PPCP_MIN_CONN_INTERVAL": 6,
    "BT_NIMBLE_SVC_GAP_PPCP_MAX_CONN_INTERVAL": 40,
    "BT_NIMBLE_SVC_GAP_PPCP_SLAVE_LATENCY": 0,
    "BT_NIMBLE_SVC_GAP_PPCP_SUPERVISION_TMO": 400,
    # The Wi-Fi driver keeps nothing in NVS, so no network it saw can come back on its own.
    "ESP_WIFI_NVS_ENABLED": False,
}
RADIO_BUILD_COMPONENTS = ("td_radio", "td_ble", "td_netlink", "td_netlink_core", "esp_wifi", "esp_netif", "lwip", "bt")
# The QEMU test image's options: it opens a pairing window at boot, logs the code, and talks over QEMU's emulated
# Ethernet. A real radio build has none of them.
TEST_ONLY_OPTIONS = ("TD_QEMU_TEST", "ETH_USE_OPENETH")
# And none of its code: the QEMU Ethernet start, which only that image compiles.
TEST_ONLY_SYMBOLS = ("td_radio_qemu_eth_start", "esp_eth_mac_new_openeth")
# The steady-state internal RAM the radio build must keep free with the station up, in bytes.
INTERNAL_FLOOR = 24576
# The radio build's static internal RAM (DIRAM in `idf.py size`), in bytes. The board measured 28815 bytes free in
# steady state at 151083 bytes static, and every static byte comes out of that margin. Raise this only with a new
# board measurement of the floor. With the Bluetooth keyboard (#276) linked and .bss moved to PSRAM the build uses
# about 136.8 KB, so the ceiling has room the board hasn't measured yet.
DIRAM_CEILING = 153600

# Functions that start BLE, put it on the air, or set who may connect. The Bluetooth keyboard needs them, so the radio
# build links them, but only BLE_OWNER may call them, and it does only when the stored setting is on. The source test
# in test_radio_guard.py holds every other file to that.
BLE_START_SYMBOLS = (
    "esp_bt_controller_init",
    "esp_bt_controller_enable",
    "esp_nimble_init",
    "nimble_port_init",
    "nimble_port_run",
    "nimble_port_freertos_init",
    "ble_hs_init",
    "ble_hs_start",
    "ble_gap_adv_start",
    "ble_gap_wl_set",
)
BLE_OWNER = "firmware/components/td_ble/td_ble_nimble.c"

# The join. Wi-Fi setup (#269) needs it, so the radio build must link it (REQUIRED_SYMBOLS), but only WIFI_JOIN_OWNER,
# the station's driver, may call it, for the one network set over USB. The source test in test_radio_guard.py holds
# every other file to that.
WIFI_JOIN_SYMBOLS = ("esp_wifi_connect",)
WIFI_JOIN_OWNER = "firmware/components/td_radio/td_radio.c"

# Functions that would scan or connect over BLE, advertise in extended mode, or scan for a Wi-Fi network. The radio
# build's image must define none of them: the linker drops every function that nothing calls, so a name missing from
# the image is a call that cannot happen.
DORMANT_SYMBOLS = (
    "ble_gap_ext_adv_start",
    "ble_gap_connect",
    "ble_gap_ext_connect",
    "ble_gap_disc",
    "ble_gap_ext_disc",
    "esp_wifi_scan_start",
    "esp_wifi_wps_start",
    "esp_wifi_80211_tx",
    "esp_smartconfig_start",
    "esp_now_init",
)
# What proves the radio build wires its station, its Bluetooth keyboard and its network service, and that the station
# joins the stored network. td_wifi_quiet_logs turns down the ESP-IDF tags that log the network's name; the linker
# keeps it only if it is called.
REQUIRED_SYMBOLS = ("td_radio_start", "esp_wifi_init", "esp_wifi_start", "esp_wifi_connect", "td_wifi_quiet_logs",
                    "td_ble_boot", "td_netlink_start")


class GuardError(ValueError):
    pass


def _settings(build):
    return json.loads((build / "config/sdkconfig.json").read_text(encoding="utf-8"))


def _components(build):
    description = json.loads((build / "project_description.json").read_text(encoding="utf-8"))
    return description["build_components"]


def _common(settings):
    if settings.get("IDF_TARGET") != TARGET:
        raise GuardError(f"IDF_TARGET is {settings.get('IDF_TARGET')!r}, not {TARGET!r}")
    if not settings.get("ESP_CONSOLE_USB_SERIAL_JTAG"):
        raise GuardError("Firmware must keep the USB-Serial/JTAG console")
    for key in IRREVERSIBLE_OPTIONS:
        if settings.get(key):
            raise GuardError(f"Irreversible eFuse or flash-encryption option enabled: {key}")


def radio_components(components):
    return sorted(name for name in components if name in RADIO_COMPONENTS or RADIO_COMPONENT_PATTERN.search(name))


def check_normal(build):
    """Fails unless the build is radio-free. Returns what it checked."""
    build = Path(build)
    settings = _settings(build)
    for key in RADIO_OPTIONS:
        if settings.get(key):
            raise GuardError(f"Radio enabled: {key}")
    _common(settings)
    components = _components(build)
    linked = radio_components(components)
    if linked:
        raise GuardError(f"Radio components linked: {linked}")
    return {"kind": "normal", "usb_serial_jtag": True, "radio_components": [], "build_components": components}


def _nm(build):
    description = json.loads((build / "project_description.json").read_text(encoding="utf-8"))
    compiler = Path(description.get("c_compiler", ""))
    candidates = [compiler.with_name(compiler.name.replace("gcc", "nm"))] if compiler.name else []
    candidates.append(Path(shutil.which(f"xtensa-{TARGET}-elf-nm") or ""))
    for nm in candidates:
        if nm.name and nm.is_file():
            return nm, build / description["app_elf"]
    raise GuardError("The toolchain's nm was not found; activate ESP-IDF first")


def defined_symbols(build):
    """Every symbol the linked image defines, read with the toolchain's nm. Absolute symbols are left out: they name
    code and data in the chip's ROM, which every build lists, the normal one too."""
    nm, elf = _nm(Path(build))
    output = subprocess.run([str(nm), "--defined-only", "--format=posix", str(elf)], check=True,
                            capture_output=True, text=True).stdout
    fields = (line.split(" ") for line in output.splitlines() if line)
    return {f[0] for f in fields if len(f) > 1 and f[1] not in ("A", "a")}


def _option_ok(value, want):
    if isinstance(want, bool):
        return bool(value) == want
    # An int option must be that int, not a bool that compares equal to it.
    return type(value) is int and value == want


def check_radio(build, symbols=None):
    """Fails unless the build is a radio build whose BLE only td_ble starts and whose station never joins. Returns
    what it checked."""
    build = Path(build)
    settings = _settings(build)
    # First, so the QEMU test image fails for what it is, not for the console it swaps.
    test_only = [key for key in TEST_ONLY_OPTIONS if settings.get(key)]
    if test_only:
        raise GuardError(f"Radio build has test-only options: {test_only}")
    _common(settings)
    for key, want in RADIO_BUILD_OPTIONS.items():
        # A missing key fails too, so a renamed option cannot pass as off.
        if key not in settings or not _option_ok(settings[key], want):
            shown = ('y' if want else 'n') if isinstance(want, bool) else want
            raise GuardError(f"Radio build needs {key}={shown}")
    floor = settings.get("TD_RADIO_INTERNAL_FLOOR")
    if not isinstance(floor, int) or floor < INTERNAL_FLOOR:
        raise GuardError(f"Radio build needs TD_RADIO_INTERNAL_FLOOR >= {INTERNAL_FLOOR}, not {floor!r}")
    components = _components(build)
    missing = [name for name in RADIO_BUILD_COMPONENTS if name not in components]
    if missing:
        raise GuardError(f"Radio build is missing components: {missing}")
    symbols = defined_symbols(build) if symbols is None else set(symbols)
    awake = [name for name in DORMANT_SYMBOLS if name in symbols]
    if awake:
        raise GuardError(f"Radio build links calls that must stay dormant: {awake}")
    test_code = [name for name in TEST_ONLY_SYMBOLS if name in symbols]
    if test_code:
        raise GuardError(f"Radio build links test-only code: {test_code}")
    absent = [name for name in REQUIRED_SYMBOLS if name not in symbols]
    if absent:
        raise GuardError(f"Radio build does not link: {absent}")
    return {"kind": "radio", "usb_serial_jtag": True, "radio_components": radio_components(components),
            "dormant": list(DORMANT_SYMBOLS), "ble_start": list(BLE_START_SYMBOLS), "ble_owner": BLE_OWNER,
            "wifi_join": list(WIFI_JOIN_SYMBOLS), "wifi_join_owner": WIFI_JOIN_OWNER,
            "build_components": components}


def check_diram(size_report, ceiling=DIRAM_CEILING):
    """Fails when `idf.py size --format json2` reports more static internal RAM than the ceiling. Returns the use."""
    report = json.loads(Path(size_report).read_text(encoding="utf-8"))
    used = next((part["used"] for part in report["layout"] if part["name"] == "DIRAM"), None)
    if used is None:
        raise GuardError("The size report has no DIRAM figure")
    if used > ceiling:
        raise GuardError(f"Radio build uses {used} bytes of static internal RAM, over the {ceiling}-byte ceiling")
    return used


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("kind", choices=("normal", "radio"))
    parser.add_argument("build", type=Path)
    parser.add_argument("--size-json", type=Path, help="radio: `idf.py size --format json2` output to hold to the "
                                                        "static internal RAM ceiling")
    args = parser.parse_args()
    try:
        result = (check_normal if args.kind == "normal" else check_radio)(args.build)
        if args.kind == "radio" and args.size_json:
            result["diram"] = check_diram(args.size_json)
            result["diram_ceiling"] = DIRAM_CEILING
    except GuardError as error:
        print(json.dumps({"ok": False, "kind": args.kind, "error": str(error)}))
        return 1
    result.pop("build_components")
    print(json.dumps({"ok": True, **result}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
