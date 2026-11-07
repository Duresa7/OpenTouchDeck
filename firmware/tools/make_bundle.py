#!/usr/bin/env python3
"""Packs a firmware build into touchdeck-<version>.tdfw, the zip the app reads to update or recover a deck.

The bundle holds bootloader.bin, partition-table.bin, touchdeck.bin and a manifest.json that names each file's flash
offset, size and SHA-256, and the image's version and elf_sha256. An update over the link sends touchdeck.bin alone
into the other OTA slot. A recovery from the cable writes every file at its offset.

    python firmware/tools/make_bundle.py [--build-dir firmware/build] [--out-dir <dir>] [--version <text>]
    python firmware/tools/make_bundle.py --check <bundle.tdfw>
"""

import argparse
import hashlib
import json
import re
import struct
import sys
import zipfile
from pathlib import Path

FORMAT = 1
PROJECT = "touchdeck"
APP_MAGIC = 0xE9
APP_DESC_OFFSET = 0x20
APP_DESC_MAGIC = 0xABCD5432
SLOT_BYTES = 0x400000
# Fixed so the same build always packs to the same bytes.
ZIP_TIME = (2020, 1, 1, 0, 0, 0)
# (name in the bundle, path under the build directory)
FILES = (
    ("bootloader.bin", "bootloader/bootloader.bin"),
    ("partition-table.bin", "partition_table/partition-table.bin"),
    ("touchdeck.bin", "touchdeck.bin"),
)
DEFAULT_OFFSETS = {"bootloader.bin": 0x0, "partition-table.bin": 0x8000}


def app_descriptor(image):
    if len(image) < APP_DESC_OFFSET + 256 or image[0] != APP_MAGIC:
        raise ValueError("touchdeck.bin is not an ESP application image")
    desc = image[APP_DESC_OFFSET:APP_DESC_OFFSET + 256]
    if struct.unpack_from("<I", desc, 0)[0] != APP_DESC_MAGIC:
        raise ValueError("touchdeck.bin has no application descriptor")

    def text(offset, size):
        return desc[offset:offset + size].split(b"\0")[0].decode("ascii", errors="replace")

    return {"version": text(16, 32), "project_name": text(48, 32), "time": text(80, 16), "date": text(96, 16),
            "idf_ver": text(112, 32), "elf_sha256": desc[144:176].hex()}


def flash_offsets(build_dir):
    """Offsets from the build's flasher_args.json, with the ESP32-S3 defaults when it is missing."""
    offsets = dict(DEFAULT_OFFSETS)
    args = build_dir / "flasher_args.json"
    if args.exists():
        files = json.loads(args.read_text(encoding="utf-8")).get("flash_files", {})
        for offset, path in files.items():
            name = Path(path).name
            if name in {"bootloader.bin", "partition-table.bin", "touchdeck.bin"}:
                offsets[name] = int(offset, 16)
    return offsets


def safe_version(version):
    cleaned = re.sub(r"[^A-Za-z0-9._-]+", "-", version).strip("-")
    return cleaned or "unknown"


def radio_build(build_dir):
    """True for the opt-in radio build (idf.py -DTD_RADIO_BUILD=1), which a release must never ship."""
    config = build_dir / "config/sdkconfig.json"
    return config.is_file() and bool(json.loads(config.read_text(encoding="utf-8")).get("TD_RADIO_BUILD"))


def make_bundle(build_dir, out_dir, version=None):
    contents = {}
    for name, relative in FILES:
        path = build_dir / relative
        if not path.is_file():
            raise FileNotFoundError(f"{path} is missing, build the firmware first")
        contents[name] = path.read_bytes()
    desc = app_descriptor(contents["touchdeck.bin"])
    if desc["project_name"] != PROJECT:
        raise ValueError(f"project is {desc['project_name']!r}, not {PROJECT!r}")
    if len(contents["touchdeck.bin"]) > SLOT_BYTES:
        raise ValueError("touchdeck.bin does not fit a 4 MB slot")
    offsets = flash_offsets(build_dir)
    version = version or desc["version"]
    manifest = {
        "format": FORMAT,
        "project": PROJECT,
        "version": version,
        "elf_sha256": desc["elf_sha256"],
        "idf_ver": desc["idf_ver"],
        "built": f"{desc['date']} {desc['time']}".strip(),
        "files": [{
            "name": name,
            "offset": hex(offsets[name]),
            "bytes": len(contents[name]),
            "sha256": hashlib.sha256(contents[name]).hexdigest(),
            # An update over the link sends the image alone, and the deck picks the slot. A cable recovery writes it
            # at `offset`, which is the ota_0 slot.
            **({"role": "update-image"} if name == "touchdeck.bin" else {}),
        } for name, _ in FILES],
    }
    # Only a radio build carries the mark, so a normal bundle's manifest stays as it was.
    if radio_build(build_dir):
        manifest["radio"] = True
    out_dir.mkdir(parents=True, exist_ok=True)
    bundle = out_dir / f"touchdeck-{safe_version(version)}.tdfw"
    with zipfile.ZipFile(bundle, "w", zipfile.ZIP_DEFLATED) as archive:
        entries = [("manifest.json", (json.dumps(manifest, indent=2) + "\n").encode("utf-8"))]
        entries += [(name, contents[name]) for name, _ in FILES]
        for name, data in entries:
            info = zipfile.ZipInfo(name, ZIP_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, data)
    return bundle, manifest


def check_bundle(path):
    """Reads a bundle back and checks every file against its manifest. Returns the manifest."""
    with zipfile.ZipFile(path) as archive:
        manifest = json.loads(archive.read("manifest.json"))
        if manifest.get("format") != FORMAT or manifest.get("project") != PROJECT:
            raise ValueError("not a TouchDeck bundle of this format")
        for entry in manifest["files"]:
            data = archive.read(entry["name"])
            if len(data) != entry["bytes"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
                raise ValueError(f"{entry['name']} does not match the manifest")
            if entry["name"] == "touchdeck.bin" and app_descriptor(data)["elf_sha256"] != manifest["elf_sha256"]:
                raise ValueError("touchdeck.bin does not match the manifest's elf_sha256")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    root = Path(__file__).resolve().parents[1]
    parser.add_argument("--build-dir", type=Path, default=root / "build")
    parser.add_argument("--out-dir", type=Path)
    parser.add_argument("--version")
    parser.add_argument("--check", type=Path, help="verify an existing bundle instead of making one")
    args = parser.parse_args()
    try:
        if args.check:
            manifest = check_bundle(args.check)
            print(json.dumps({"ok": True, "bundle": str(args.check), "version": manifest["version"],
                              "elf_sha256": manifest["elf_sha256"]}))
            return 0
        bundle, manifest = make_bundle(args.build_dir, args.out_dir or args.build_dir, args.version)
        check_bundle(bundle)
        print(json.dumps({"ok": True, "bundle": str(bundle), "bytes": bundle.stat().st_size,
                          "version": manifest["version"], "elf_sha256": manifest["elf_sha256"]}))
        return 0
    except (OSError, ValueError, KeyError, zipfile.BadZipFile) as error:
        print(json.dumps({"ok": False, "error": str(error)}))
        return 1


if __name__ == "__main__":
    sys.exit(main())
