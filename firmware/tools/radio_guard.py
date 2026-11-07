#!/usr/bin/env python3
"""Checks a firmware build directory against the rules for its kind: the normal build or the radio build.

The normal build is what every release ships. It must stay radio-free: no Wi-Fi or Bluetooth option on, and no radio
or network component linked. The release recipe (app/scripts/package_windows.py) runs the same check before it packs
the firmware, so a mistake here cannot ship radios to everyone.

The radio build is the opt-in build that `idf.py -DTD_RADIO_BUILD=1` makes (see docs/firmware.md). It must have Wi-Fi,
lwIP and NimBLE, keep Wi-Fi and lwIP buffers in PSRAM and the BLE controller in flash, and keep BLE dormant: until an
opt-in setting starts it, nothing in the linked image may start the BLE controller or host, advertise, scan or
connect, and the Wi-Fi station may join the one network set over USB (the `wifi.*` link messages) but never scan.

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

# Options that turn a radio or the radio build on. Any one of them fails the normal build.
RADIO_OPTIONS = (
    "ESP_WIFI_ENABLED",
    "ESP_HOST_WIFI_ENABLED",
    "BT_ENABLED",
    "BT_CONTROLLER_ENABLED",
    "BT_NIMBLE_ENABLED",
    "BT_BLUEDROID_ENABLED",
    "TD_RADIO_BUILD",
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
    "BT_CTRL_RUN_IN_FLASH_ONLY": True,
    # No scanning or connecting role is even compiled in.
    "BT_NIMBLE_ROLE_OBSERVER": False,
    "BT_NIMBLE_ROLE_CENTRAL": False,
    # The Wi-Fi driver keeps nothing in NVS, so no network it saw can come back on its own.
    "ESP_WIFI_NVS_ENABLED": False,
}
RADIO_BUILD_COMPONENTS = ("td_radio", "esp_wifi", "esp_netif", "lwip", "bt")
# The steady-state internal RAM the radio build must keep free with the station up, in bytes.
INTERNAL_FLOOR = 24576
# The radio build's static internal RAM (DIRAM in `idf.py size`), in bytes. The board measured 28815 bytes free in
# steady state at 151083 bytes static, and every static byte comes out of that margin. Raise this only with a new
# board measurement of the floor.
DIRAM_CEILING = 153600

# Functions that would start BLE, put it on the air, or scan for a Wi-Fi network. The radio build's image must define
# none of them: the linker drops every function that nothing calls, so a name missing from the image is a call that
# cannot happen. A later ticket that adds an opt-in setting, default off, moves the BLE names out of this list.
# esp_wifi_connect left it with Wi-Fi setup (#269): the station joins the network set over USB.
DORMANT_SYMBOLS = (
    "esp_bt_controller_init",
    "esp_bt_controller_enable",
    "esp_nimble_init",
    "nimble_port_init",
    "nimble_port_run",
    "nimble_port_freertos_init",
    "ble_hs_init",
    "ble_hs_start",
    "ble_gap_adv_start",
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
# What proves the radio build wires its station and joins the stored network.
REQUIRED_SYMBOLS = ("td_radio_start", "esp_wifi_init", "esp_wifi_start", "esp_wifi_connect")


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


def check_radio(build, symbols=None):
    """Fails unless the build is a radio build with dormant BLE and an unjoined station. Returns what it checked."""
    build = Path(build)
    settings = _settings(build)
    _common(settings)
    for key, want in RADIO_BUILD_OPTIONS.items():
        # A missing key fails too, so a renamed option cannot pass as off.
        if key not in settings or bool(settings[key]) != want:
            raise GuardError(f"Radio build needs {key}={'y' if want else 'n'}")
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
    absent = [name for name in REQUIRED_SYMBOLS if name not in symbols]
    if absent:
        raise GuardError(f"Radio build does not link: {absent}")
    return {"kind": "radio", "usb_serial_jtag": True, "radio_components": radio_components(components),
            "dormant": list(DORMANT_SYMBOLS), "build_components": components}


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
