#!/usr/bin/env python3
"""Serial harness for verifying TouchDeck firmware on the ESP32-S3 USB-Serial/JTAG port.

Run it with the ESP-IDF Python (it needs pyserial and esptool), normally through
touchdeck.ps1:  pwsh -NoProfile -File touchdeck.ps1 serial <command> [options]

Commands:
  ports    List Espressif USB devices.
  doctor   Read-only check: port free, flashed app == local build, firmware alive.
  capture  Reset the board (optional), record the serial log, check expectations.
  cmd      Send one console line and print the matching "@td" JSON reply (M1-03 and later).
  screenshot  Stream the display over the console and save it as a PNG (M1-04 and later).
  keys     Read the key engine's record over the console and save it as JSON (M1-09 and later).
  store    Put a file on the deck's storage partition or get one back, checking its CRC-32 (M2-03 and later).
  unit     Reset the board running firmware/test_app and report its Unity summary.
"""
import argparse
import base64
import hashlib
import json
import os
import re
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib
from collections import deque
from datetime import datetime, timezone
from pathlib import Path

import serial
from esp_pylib.serial_reset import hard_reset as esp_hard_reset
from serial.tools import list_ports
from touchdeck_link import StreamDecoder, message

ESPRESSIF_VID = 0x303A
USB_SERIAL_JTAG_PID = 0x1001
BAUD = 115200
REPO_ROOT = Path(__file__).resolve().parents[4]
# touchdeck.ps1 sets TOUCHDECK_PROJECT to the ESP-IDF project of the checkout under test.
BUILD_BIN = Path(os.environ.get("TOUCHDECK_PROJECT", REPO_ROOT)) / "build" / "touchdeck.bin"
PARTITION_TABLE_OFFSET = 0x8000
PARTITION_TABLE_SIZE = 0xC00
DEFAULT_APP_OFFSET = 0x10000  # factory slot of the pre-M1-02 single-app layout
APP_DESC_OFFSET = 0x20        # esp_app_desc_t follows the image + first segment headers
OTADATA_OFFSET = 0xF000       # two 4 KB sectors, one esp_ota_select_entry_t at the start of each
OTA_SECTOR_SIZE = 0x1000
FLASH_BLOCK_OFFSET = PARTITION_TABLE_OFFSET
FLASH_BLOCK_SIZE = OTADATA_OFFSET + 2 * OTA_SECTOR_SIZE - PARTITION_TABLE_OFFSET  # the table through otadata
OTA_STATES = {0: "NEW", 1: "PENDING_VERIFY", 2: "VALID", 3: "INVALID", 4: "ABORTED", 0xFFFFFFFF: "UNDEFINED"}
APP_DESC_MAGIC = 0xABCD5432
BANNER = "touchdeck: TouchDeck firmware starting"
# Older firmware can lose an echo newline, leaving a reply after a partial command.
REPLY_RE = re.compile(r"^.*?@td (\{.*\})\s*$")
TDB_RE = re.compile(rb"^(?:td> )*@tdb (\S*)\s*$")
TDR_RE = re.compile(rb"^(?:td> )*@tdr (\{.*\})\s*$")
TDS_RE = re.compile(rb"^(?:td> )*@tds (\S*)\s*$")
TDS_LOG_RE = re.compile(r"@tds (\S+)\s*$")
STORE_LINE_RAW = 180
STORE_PUT_MAX = 65536
UNITY_SUMMARY_RE = r"(\d+) Tests (\d+) Failures (\d+) Ignored"
UNITY_RESULT_RE = r"^(OK|FAIL)\s*$"


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def espressif_ports():
    return [p for p in list_ports.comports() if p.vid == ESPRESSIF_VID]


def resolve_port(port):
    port = port or os.environ.get("TOUCHDECK_PORT")
    if port:
        return port
    ports = [p.device for p in espressif_ports() if p.pid == USB_SERIAL_JTAG_PID]
    if len(ports) != 1:
        raise SystemExit(f"error: expected one Espressif USB-Serial/JTAG port, found {ports}. Pass --port or set TOUCHDECK_PORT.")
    return ports[0]


def open_port(device):
    ser = serial.Serial()
    ser.port = device
    ser.baudrate = BAUD
    ser.timeout = 0.2
    # Firmware without a console never drains the port, so a write would block forever.
    ser.write_timeout = 2
    # Released lines keep GPIO0 high (normal boot) and EN high (no reset) while opening.
    ser.dtr = False
    ser.rts = False
    ser.open()
    return ser


def hard_reset(ser):
    # esptool's own USB-Serial/JTAG timings. The device drops off USB while EN is low,
    # so the release needs the hold and settle delays before any further line writes.
    esp_hard_reset(ser, hold_delay=0.2, post_release_delay=0.2)


def hard_reset_reading(ser):
    """Reset the board and return the bytes it printed during the reset.

    The reset takes 0.4 s, and the chip boots and starts printing within it. On the dev PC the port holds about
    12 KB for a reader that is not there. Once the host has not collected data for 50 ms, the chip drops every byte
    that does not fit, so a program that prints more than that at once, such as the unit test app, loses its tail.
    Reading through the reset keeps the buffer empty.
    """
    got = []
    stop = threading.Event()

    def pump():
        while not stop.is_set():
            try:
                data = ser.read(max(1, ser.in_waiting))
            except serial.SerialException:
                return
            if data:
                got.append(data)

    thread = threading.Thread(target=pump, daemon=True)
    thread.start()
    try:
        hard_reset(ser)
    finally:
        stop.set()
        thread.join()
    return b"".join(got)


def wait_for_port(device, timeout=5.0, settle=1.0):
    # After esptool or a reset, the port re-enumerates. Opening it mid-way and toggling
    # lines has wedged the Windows driver until a replug, so wait for it to settle.
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if any(p.device == device for p in list_ports.comports()):
            time.sleep(settle)
            return True
        time.sleep(0.25)
    return False


def reopen(device, deadline):
    while time.monotonic() < deadline:
        time.sleep(0.25)
        try:
            return open_port(device)
        except serial.SerialException:
            continue
    return None


def record(device, seconds, expects, transcript, reset=True, stop_when_matched=False,
           sends=(), send_after=1.0, send_times=1, send_every=0.0):
    """Read the port for `seconds`, matching `expects` regexes in order.

    Returns (matches, lines). Each match is {pattern, line, t} or {pattern, line: None}.
    """
    patterns = [re.compile(p) for p in expects]
    matches = [{"pattern": p.pattern, "line": None, "t": None} for p in patterns]
    next_expect = 0
    lines = []
    ser = open_port(device)
    start = time.monotonic()
    deadline = start + seconds
    pending = hard_reset_reading(ser) if reset else b""
    buf = b""
    decoder = StreamDecoder()
    next_send = start + send_after
    sends_left = send_times if sends else 0
    with open(transcript, "w", encoding="utf-8") as out:
        while time.monotonic() < deadline:
            if sends_left and time.monotonic() >= next_send:
                for line in sends:
                    t = round(time.monotonic() - start, 3)
                    try:
                        ser.write(line if isinstance(line, bytes) else line.encode("utf-8") + b"\n")
                        out.write(f"[+{t:7.3f}s] >> sent {line!r}\n")
                    except serial.SerialException as e:
                        out.write(f"[+{t:7.3f}s] >> send failed {line!r}: {e}\n")
                        log(f"error: could not send {line!r}: {e}")
                sends_left -= 1
                next_send += send_every
            try:
                chunk, pending = pending or ser.read(4096), b""
            except serial.SerialException:
                # The port can vanish for a moment around a chip reset.
                ser.close()
                ser = reopen(device, deadline)
                if ser is None:
                    break
                continue
            complete = []
            for kind, value in decoder.feed_events(chunk):
                if kind == "text":
                    buf += value
                    *ready, buf = buf.split(b"\n")
                    complete.extend(ready)
                else:
                    try:
                        value = message(value)
                    except ValueError:
                        pass
                    complete.append(b"@tdlink " + json.dumps(value).encode("utf-8"))
            for raw in complete:
                text = raw.decode("utf-8", errors="replace").rstrip("\r")
                t = round(time.monotonic() - start, 3)
                out.write(f"[+{t:7.3f}s] {text}\n")
                out.flush()
                lines.append(text)
                if next_expect < len(patterns) and patterns[next_expect].search(text):
                    matches[next_expect].update(line=text, t=t)
                    next_expect += 1
            if stop_when_matched and next_expect == len(patterns):
                break
    if ser is not None:
        ser.close()
    return matches, lines


def parse_app_desc(blob):
    magic, = struct.unpack_from("<I", blob, 0)
    if magic != APP_DESC_MAGIC:
        return None
    def text(off, size):
        return blob[off:off + size].split(b"\0")[0].decode("ascii", errors="replace")
    return {
        "version": text(16, 32),
        "project_name": text(48, 32),
        "time": text(80, 16),
        "date": text(96, 16),
        "idf_ver": text(112, 32),
        "elf_sha256": blob[144:176].hex(),
    }


def local_app_desc():
    if not BUILD_BIN.exists():
        return None
    data = BUILD_BIN.read_bytes()
    return parse_app_desc(data[APP_DESC_OFFSET:APP_DESC_OFFSET + 256])


def partition_entries(table):
    """The partition table's entries as {label: {type, subtype, offset, size}}, in table order."""
    entries = {}
    for off in range(0, len(table) - 31, 32):
        magic, ptype, subtype, offset, size = struct.unpack_from("<HBBII", table, off)
        if magic != 0x50AA:
            break
        label = table[off + 12:off + 28].split(b"\0")[0].decode("ascii", errors="replace")
        entries.setdefault(label, {"type": ptype, "subtype": subtype, "offset": offset, "size": size})
    return entries


def app_offset(table):
    """Offset of the app slot the bootloader runs when nothing says otherwise: ota_0, else factory, else the first app."""
    entries = partition_entries(table)
    apps = {label: e["offset"] for label, e in entries.items() if e["type"] == 0}
    return apps.get("ota_0", apps.get("factory", next(iter(apps.values()), DEFAULT_APP_OFFSET)))


def ota_select_crc(ota_seq):
    """esp_rom_crc32_le(UINT32_MAX, &ota_seq, 4), which is zlib's crc32 started from 0xFFFFFFFF."""
    return zlib.crc32(struct.pack("<I", ota_seq), 0xFFFFFFFF) & 0xFFFFFFFF


def parse_otadata(blob):
    """The two otadata entries: [{sector, ota_seq, state, crc_ok, valid}]. `valid` is what the bootloader can pick."""
    entries = []
    for sector in range(2):
        ota_seq, _label, state, crc = struct.unpack_from("<I20sII", blob, sector * OTA_SECTOR_SIZE)
        crc_ok = ota_seq != 0xFFFFFFFF and crc == ota_select_crc(ota_seq)
        name = OTA_STATES.get(state, f"0x{state:x}")
        entries.append({"sector": sector, "ota_seq": None if ota_seq == 0xFFFFFFFF else ota_seq, "state": name,
                        "crc_ok": crc_ok, "valid": crc_ok and name not in ("INVALID", "ABORTED")})
    return entries


def boot_slot_label(otadata, ota_labels):
    """The slot the bootloader runs: the valid entry with the higher ota_seq picks (seq - 1) % slots, and with none
    the first OTA slot runs, which is what a fresh flash of ota_data_initial.bin gives."""
    valid = [e for e in otadata if e["valid"]]
    if not valid or not ota_labels:
        return ota_labels[0] if ota_labels else None
    best = max(valid, key=lambda e: e["ota_seq"])
    return ota_labels[(best["ota_seq"] - 1) % len(ota_labels)]


def read_flash(device, offset, size):
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "region.bin"
        cmd = [sys.executable, "-m", "esptool", "--port", device, "--after", "hard-reset",
               "read-flash", hex(offset), str(size), str(out)]
        result = subprocess.run(cmd, capture_output=True, text=True)
        wait_for_port(device)
        if result.returncode != 0:
            return None, (result.stdout + result.stderr).strip().splitlines()[-1:]
        return out.read_bytes(), None


def flashed_layout(device):
    """Reads the table and otadata in one go, then the descriptor of the slot the bootloader runs.

    Returns ({boot_slot, otadata, app}, error). `app` is the app descriptor of the boot slot. Without OTA slots it is
    the one the old rule picks and boot_slot is None."""
    block, err = read_flash(device, FLASH_BLOCK_OFFSET, FLASH_BLOCK_SIZE)
    if err:
        return None, err
    table = block[:PARTITION_TABLE_SIZE]
    entries = partition_entries(table)
    ota = sorted((label for label, e in entries.items() if e["type"] == 0 and 0x10 <= e["subtype"] <= 0x1F),
                 key=lambda label: entries[label]["subtype"])
    result = {"boot_slot": None, "otadata": None, "app": None}
    offset = app_offset(table)
    otadata_entry = entries.get("otadata")
    if ota and otadata_entry and otadata_entry["offset"] == OTADATA_OFFSET:
        result["otadata"] = parse_otadata(block[OTADATA_OFFSET - FLASH_BLOCK_OFFSET:])
        result["boot_slot"] = boot_slot_label(result["otadata"], ota)
        offset = entries[result["boot_slot"]]["offset"]
    blob, err = read_flash(device, offset + APP_DESC_OFFSET, 256)
    if err:
        return result, err
    result["app"] = parse_app_desc(blob)
    return result, None


def flashed_app_desc(device):
    layout, err = flashed_layout(device)
    return (layout["app"] if layout else None), err


def cmd_ports(_args):
    print(json.dumps([
        {"device": p.device, "vid": f"{p.vid:04x}", "pid": f"{p.pid:04x}",
         "kind": "usb-serial-jtag" if p.pid == USB_SERIAL_JTAG_PID else "other",
         "description": p.description}
        for p in espressif_ports()
    ], indent=2))
    return 0


def cmd_doctor(args):
    report = {"port": None, "port_free": False, "local_build": None, "flashed_app": None, "boot_slot": None,
              "otadata": None, "build_matches": False, "banner_seen": False, "alive_seen": False, "ok": False}
    try:
        device = resolve_port(args.port)
    except SystemExit as e:
        report["error"] = str(e)
        print(json.dumps(report, indent=2))
        return 1
    report["port"] = device
    try:
        open_port(device).close()
        report["port_free"] = True
    except serial.SerialException as e:
        report["error"] = f"cannot open {device} (another monitor may hold it): {e}"
        print(json.dumps(report, indent=2))
        return 1

    report["local_build"] = local_app_desc()
    layout, err = flashed_layout(device)
    flashed = layout["app"] if layout else None
    report["flashed_app"] = flashed
    if layout:
        report["boot_slot"] = layout["boot_slot"]
        report["otadata"] = layout["otadata"]
    if err:
        report["error"] = f"esptool read-flash failed: {err}"
    if flashed and report["local_build"]:
        report["build_matches"] = flashed["elf_sha256"] == report["local_build"]["elf_sha256"]

    # Holding the port open while foreign firmware boots wedged the Waveshare factory
    # demo's USB until a replug, so only watch the log when the board runs TouchDeck.
    if not flashed or flashed["project_name"] != "touchdeck":
        report["error"] = report.get("error") or "board is not running touchdeck firmware; skipped serial check"
        print(json.dumps(report, indent=2))
        return 2

    with tempfile.TemporaryDirectory() as tmp:
        matches, _ = record(device, args.seconds, [re.escape(BANNER), r"touchdeck: alive \d+"],
                            Path(tmp) / "doctor.log", reset=True, stop_when_matched=True)
    report["banner_seen"] = matches[0]["line"] is not None
    report["alive_seen"] = matches[1]["line"] is not None
    report["ok"] = report["build_matches"] and report["banner_seen"] and report["alive_seen"]
    print(json.dumps(report, indent=2))
    return 0 if report["ok"] else 2


def cmd_capture(args):
    if args.send_times < 1 or (args.send_times > 1 and args.send_every <= 0):
        log("error: --send-times must be at least 1, and above 1 it needs a positive --send-every")
        return 2
    device = resolve_port(args.port)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    transcript = out_dir / "serial.log"
    started = datetime.now(timezone.utc).isoformat(timespec="seconds")
    matches, lines = record(device, args.seconds, args.expect or [], transcript,
                            reset=not args.no_reset, stop_when_matched=args.stop_when_matched,
                            sends=args.send or [], send_after=args.send_after,
                            send_times=args.send_times, send_every=args.send_every)
    summary = {
        "port": device,
        "reset": not args.no_reset,
        "started_utc": started,
        "seconds": args.seconds,
        "line_count": len(lines),
        "local_build": local_app_desc(),
        "expectations": matches,
        "ok": all(m["line"] is not None for m in matches),
        "transcript": str(transcript),
    }
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0 if summary["ok"] else 1


def console_word(line):
    """The first word as the firmware's esp_console_split_argv parses it, or None for no word.

    It splits on spaces only, honors double quotes, and keeps an escaped backslash,
    quote, or space while dropping any other escaped character.
    """
    word, state = [], "space"
    for ch in line:
        if state == "space":
            if ch == " ":
                continue
            if ch == '"':
                state = "quoted"
            elif ch == "\\":
                state = "arg_escaped"
            else:
                state = "arg"
                word.append(ch)
        elif state in ("arg_escaped", "quoted_escaped"):
            if ch in '\\" ':
                word.append(ch)
            state = state.removesuffix("_escaped")
        elif ch == "\\":
            state += "_escaped"
        elif (state == "quoted" and ch == '"') or (state == "arg" and ch == " "):
            return "".join(word)
        else:
            word.append(ch)
    return None if state == "space" else "".join(word)


def run_command(ser, line, timeout):
    name = console_word(line)
    ser.reset_input_buffer()
    ser.write(line.encode("utf-8") + b"\n")
    ser.flush()
    start = time.monotonic()
    deadline = start + timeout
    buf = b""
    decoder = StreamDecoder()
    while time.monotonic() < deadline:
        text, _frames = decoder.feed(ser.read(ser.in_waiting or 1))
        buf += text
        *complete, buf = buf.split(b"\n")
        for raw in complete:
            match = REPLY_RE.match(raw.decode("utf-8", errors="replace").rstrip("\r"))
            if not match:
                continue
            try:
                reply = json.loads(match.group(1))
            except ValueError:
                continue
            # The firmware echoes at most the first 63 characters of the command word.
            echoed = reply.get("cmd") or ""
            if echoed == name or (len(echoed) >= 32 and name.startswith(echoed)):
                return reply, time.monotonic() - start
    return None, time.monotonic() - start


def cmd_cmd(args):
    if console_word(args.line) is None:
        log("error: empty command line")
        return 2
    try:
        device = resolve_port(args.port)
    except SystemExit as e:
        log(str(e))
        return 2
    try:
        ser = open_port(device)
    except serial.SerialException as e:
        log(f"error: cannot open {device}: {e}")
        return 2
    ser.write_timeout = args.timeout
    code = 0
    try:
        for i in range(args.repeat):
            if i:
                time.sleep(args.interval)
            reply, seconds = run_command(ser, args.line, args.timeout)
            if reply is None:
                log(f"error: no reply to {console_word(args.line)!r} within {args.timeout}s on {device}")
                return 2
            print(json.dumps(reply))
            if args.time:
                log(f"rtt_ms: {seconds * 1000:.1f}")
            if reply.get("ok") is not True:
                code = 1
    except serial.SerialTimeoutException:
        log(f"error: {device} did not accept the line within {args.timeout}s (firmware without a console?)")
        return 2
    except serial.SerialException as e:
        log(f"error: lost {device}: {e}")
        return 2
    finally:
        try:
            ser.close()
        except serial.SerialException:
            pass
    return code


class Console:
    """Line reader over the open port. The reply to `late_cmd` is set aside in `late_reply`."""

    def __init__(self, ser, transcript=None):
        self.ser = ser
        self.tail = b""
        self.lines = deque()
        self.late_cmd = None
        self.late_reply = None
        self.transcript = transcript
        self.start = time.monotonic()
        self.decoder = StreamDecoder()

    def note(self, text):
        if self.transcript:
            self.transcript.write(f"[+{time.monotonic() - self.start:7.3f}s] {text}\n")

    def send(self, line, note=True):
        self.ser.write(line.encode("utf-8") + b"\n")
        self.ser.flush()
        if note:
            self.note(f">> sent {line!r}")

    def next_event(self, deadline):
        """Next ("tdb", base64 bytes), ("tdr", JSON text), ("tds", base64 bytes), ("reply", dict) or ("late", dict).

        None at the deadline. The console echoes each @tds line a put sends, so a put sees them as "tds" events too.
        """
        while True:
            while not self.lines:
                if time.monotonic() >= deadline:
                    return None
                # Take everything queued so a 1.6 MB stream is not read a byte at a time.
                text, _frames = self.decoder.feed(self.ser.read(self.ser.in_waiting or 1))
                if text:
                    *complete, self.tail = (self.tail + text).split(b"\n")
                    self.lines.extend(complete)
            raw = self.lines.popleft().rstrip(b"\r")
            match = TDB_RE.match(raw)
            if match:
                return "tdb", match.group(1)
            match = TDR_RE.match(raw)
            if match:
                return "tdr", match.group(1).decode("utf-8", errors="replace")
            match = TDS_RE.match(raw)
            if match:
                return "tds", match.group(1)
            text = raw.decode("utf-8", errors="replace")
            self.note(text)
            match = REPLY_RE.match(text)
            if not match:
                continue
            try:
                reply = json.loads(match.group(1))
            except ValueError:
                continue
            echoed = reply.get("cmd") or ""
            # The firmware echoes at most the first 63 characters of the command word.
            if (self.late_cmd is not None and self.late_reply is None
                    and (echoed == self.late_cmd or (len(echoed) >= 32 and self.late_cmd.startswith(echoed)))):
                self.late_reply = reply
                return "late", reply
            return "reply", reply


def request_screenshot(console, scale, timeout):
    """Request one screenshot and verify it.

    Returns (shot, raw, failure). `raw` is the RGB565 bytes when every check passed.
    `failure` is None, "check" (bad data), "device" (firmware said ok false) or "timeout".
    """
    shot = {"ok": False, "scale": scale, "width": None, "height": None, "format": None, "bytes": None,
            "lines": None, "crc32": None, "crc_ok": None, "seconds": None, "bytes_per_s": None}
    console.send(f"screenshot {scale}")
    start = time.monotonic()
    deadline = start + timeout
    header = trailer = None
    chunks, problems, seen = [], [], 0
    while trailer is None:
        event = console.next_event(deadline)
        if event is None:
            shot["error"] = (f"no screenshot reply within {timeout}s" if header is None
                             else f"stream stalled after {seen} of {shot['lines']} lines")
            return shot, None, "timeout"
        kind, value = event
        if kind == "tdb":
            if header is None:
                continue
            seen += 1
            try:
                # Without validate=True b64decode silently drops stray characters.
                chunks.append(base64.b64decode(value, validate=True))
            except ValueError:
                problems.append(f"line {seen} is not valid base64")
        elif kind in ("reply", "late") and value.get("cmd") == "screenshot":
            if value.get("ok") is not True:
                shot["error"] = value.get("error") or "firmware reported ok false"
                return shot, None, "device"
            if value.get("phase") == "start" and header is None:
                header = value
                for key in ("width", "height", "format", "bytes", "lines"):
                    shot[key] = header.get(key)
            elif value.get("phase") == "end" and header is not None:
                trailer = value
    seconds = time.monotonic() - start
    raw = b"".join(chunks)
    crc = zlib.crc32(raw)
    width, height, size, lines = (shot[key] for key in ("width", "height", "bytes", "lines"))
    if shot["format"] != "rgb565":
        problems.append(f"format {shot['format']!r}, expected 'rgb565'")
    if header.get("scale") != scale:
        problems.append(f"header scale {header.get('scale')!r}, requested {scale}")
    if not all(isinstance(v, int) for v in (width, height, size, lines)):
        problems.append("header lacks integer width, height, bytes and lines")
    else:
        if size != width * height * 2:
            problems.append(f"header says {size} bytes for {width}x{height} rgb565")
        if len(raw) != size:
            problems.append(f"got {len(raw)} of {size} bytes")
        if seen != lines:
            problems.append(f"got {seen} of {lines} lines")
    crc_ok = crc == trailer.get("crc32")
    if not crc_ok:
        problems.append(f"crc32 of data is {crc}, trailer says {trailer.get('crc32')}")
    shot.update(ok=not problems, crc32=trailer.get("crc32"), crc_ok=crc_ok, seconds=round(seconds, 3),
                bytes_per_s=round(len(raw) / max(seconds, 1e-6)))
    if problems:
        shot["error"] = "; ".join(problems)
        return shot, None, "check"
    return shot, raw, None


def take_screenshot(console, scale, timeout, retries):
    """request_screenshot, asking again after a "check" failure up to `retries` times."""
    attempts = 0
    while True:
        attempts += 1
        shot, raw, failure = request_screenshot(console, scale, timeout)
        shot["attempts"] = attempts
        if failure != "check" or attempts > retries:
            return shot, raw, failure
        log(f"warning: attempt {attempts} failed ({shot['error']}), requesting the screenshot again")


def rgb565_to_rgb888(value):
    r5, g6, b5 = value >> 11, (value >> 5) & 0x3F, value & 0x1F
    # Repeating the top bits maps full scale to 255, and >>3, >>2, >>3 gives r5, g6, b5 back.
    return bytes(((r5 << 3) | (r5 >> 2), (g6 << 2) | (g6 >> 4), (b5 << 3) | (b5 >> 2)))


def encode_png(width, height, rgb565):
    pixels = struct.unpack(f"<{width * height}H", rgb565)
    palette = {value: rgb565_to_rgb888(value) for value in set(pixels)}
    rgb = b"".join(map(palette.__getitem__, pixels))
    stride = width * 3
    scanlines = b"".join(b"\0" + rgb[y:y + stride] for y in range(0, len(rgb), stride))

    def chunk(tag, body):
        return struct.pack(">I", len(body)) + tag + body + struct.pack(">I", zlib.crc32(tag + body))

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(scanlines))
            + chunk(b"IEND", b""))


def save_numbered(path, i, count, payload):
    """Write payload to `path`, or to base-i.ext when taking several shots. Returns the path used."""
    path = Path(path)
    if count > 1:
        path = path.with_name(f"{path.stem}-{i}{path.suffix}")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(payload)
    return str(path)


def cmd_screenshot(args):
    if not (args.out or args.raw):
        log("error: pass --out FILE.png or --raw FILE")
        return 2
    if args.count < 1 or args.retries < 0:
        log("error: --count must be at least 1 and --retries at least 0")
        return 2
    if args.background is not None and console_word(args.background) is None:
        log("error: empty --background line")
        return 2
    try:
        device = resolve_port(args.port)
    except SystemExit as e:
        log(str(e))
        return 2
    try:
        ser = open_port(device)
    except serial.SerialException as e:
        log(f"error: cannot open {device}: {e}")
        return 2
    ser.write_timeout = args.timeout
    transcript = None
    if args.log:
        Path(args.log).parent.mkdir(parents=True, exist_ok=True)
        transcript = open(args.log, "w", encoding="utf-8")
    console = Console(ser, transcript)
    shots, timed_out = [], False
    try:
        # Only once: a reset between shots would drop a background reply that arrived meanwhile.
        ser.reset_input_buffer()
        if args.background:
            console.late_cmd = console_word(args.background)
            late_deadline = time.monotonic() + args.background_timeout
            console.send(args.background)
        for i in range(1, args.count + 1):
            shot, raw, failure = take_screenshot(console, args.scale, args.timeout, args.retries)
            shot["out"] = None
            if raw is not None and args.out:
                shot["out"] = save_numbered(args.out, i, args.count, encode_png(shot["width"], shot["height"], raw))
            if args.raw:
                shot["raw"] = save_numbered(args.raw, i, args.count, raw) if raw is not None else None
            if failure:
                log(f"error: shot {i}/{args.count}: {shot['error']}")
            else:
                log(f"shot {i}/{args.count}: {shot['width']}x{shot['height']}, {shot['bytes']} bytes in "
                    f"{shot['seconds']}s ({shot['bytes_per_s'] // 1024} KiB/s), crc ok")
            shots.append(shot)
            if failure == "timeout":
                # The board may still be streaming, so a further request would interleave with it.
                timed_out = True
                break
        while args.background and not timed_out and console.late_reply is None:
            if console.next_event(late_deadline) is None:
                log(f"error: no {console.late_cmd!r} reply within {args.background_timeout}s of sending it")
                break
    except serial.SerialTimeoutException:
        log(f"error: {device} did not accept the line within {args.timeout}s (firmware without a console?)")
        return 2
    except serial.SerialException as e:
        log(f"error: lost {device}: {e}")
        return 2
    finally:
        if transcript:
            transcript.close()
        try:
            ser.close()
        except serial.SerialException:
            pass
    ok = not timed_out and all(shot["ok"] for shot in shots)
    result = {"ok": ok, "port": device, "scale": args.scale}
    if args.count == 1:
        result.update({k: v for k, v in shots[0].items() if k != "ok"})
    else:
        result["shots"] = shots
    if args.background:
        background_ok = console.late_reply is not None and console.late_reply.get("ok") is True
        result.update(ok=ok and background_ok, background=console.late_reply, background_ok=background_ok)
    print(json.dumps(result, indent=2))
    if timed_out:
        return 2
    return 0 if result["ok"] else 1


def brief_report(report):
    """A report as the plan writes it: `kbd <mods> [<keys>]` or `con <usage>`."""
    if report.get("type") == "con":
        return f"con {report.get('usage')}"
    keys = ",".join(str(k) for k in report.get("keys") or [])
    return f"kbd {report.get('mods')} [{keys}]"


def read_record(console, timeout):
    """Request `keys record dump` and collect the record.

    Returns (result, failure). `failure` is None, "check" (the count or a line did not check out), "device"
    (firmware said ok false) or "timeout".
    """
    result = {"ok": False, "count": None, "dropped": None, "lines": 0, "reports": []}
    console.send("keys record dump")
    deadline = time.monotonic() + timeout
    header = trailer = None
    problems = []
    while trailer is None:
        event = console.next_event(deadline)
        if event is None:
            result["error"] = (f"no keys record reply within {timeout}s" if header is None
                               else f"stream stalled after {result['lines']} of {result['count']} lines")
            return result, "timeout"
        kind, value = event
        if kind == "tdr":
            if header is None:
                continue
            result["lines"] += 1
            try:
                result["reports"].append(json.loads(value))
            except ValueError:
                problems.append(f"line {result['lines']} is not valid JSON")
        elif kind in ("reply", "late") and value.get("cmd") == "keys" and value.get("op") == "record":
            if value.get("ok") is not True:
                result["error"] = value.get("error") or "firmware reported ok false"
                return result, "device"
            if value.get("phase") == "start" and header is None:
                header = value
                result["count"] = header.get("count")
                result["dropped"] = header.get("dropped")
            elif value.get("phase") == "end" and header is not None:
                trailer = value
    count = result["count"]
    if not (isinstance(count, int) and isinstance(result["dropped"], int)):
        problems.append("start reply lacks integer count and dropped")
    else:
        if result["lines"] != count:
            problems.append(f"got {result['lines']} of {count} lines")
        if trailer.get("count") != count:
            problems.append(f"end reply says {trailer.get('count')} lines, start said {count}")
    for i, report in enumerate(result["reports"], 1):
        if not (isinstance(report.get("seq"), int) and isinstance(report.get("t"), int)
                and report.get("type") in ("kbd", "con")):
            problems.append(f"line {i} lacks seq, t and a kbd or con type")
            break
    result["brief"] = [brief_report(report) for report in result["reports"]]
    result["ok"] = not problems
    if problems:
        result["error"] = "; ".join(problems)
        return result, "check"
    return result, None


def cmd_keys(args):
    try:
        device = resolve_port(args.port)
    except SystemExit as e:
        log(str(e))
        return 2
    try:
        ser = open_port(device)
    except serial.SerialException as e:
        log(f"error: cannot open {device}: {e}")
        return 2
    ser.write_timeout = args.timeout
    transcript = None
    if args.log:
        Path(args.log).parent.mkdir(parents=True, exist_ok=True)
        transcript = open(args.log, "w", encoding="utf-8")
    console = Console(ser, transcript)
    try:
        ser.reset_input_buffer()
        result, failure = read_record(console, args.timeout)
    except serial.SerialTimeoutException:
        log(f"error: {device} did not accept the line within {args.timeout}s (firmware without a console?)")
        return 2
    except serial.SerialException as e:
        log(f"error: lost {device}: {e}")
        return 2
    finally:
        if transcript:
            transcript.close()
        try:
            ser.close()
        except serial.SerialException:
            pass
    status = {"ok": result["ok"], "port": device, "count": result["count"], "dropped": result["dropped"],
              "lines": result["lines"], "out": None}
    if failure is None:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        record = {"count": result["count"], "dropped": result["dropped"], "reports": result["reports"],
                  "brief": result["brief"]}
        out.write_text(json.dumps(record, indent=2), encoding="utf-8")
        status["out"] = str(out)
        log(f"record: {result['count']} reports, {result['dropped']} dropped, written to {out}")
    else:
        status["error"] = result["error"]
        log(f"error: {result['error']}")
    status["brief"] = result.get("brief", [])
    print(json.dumps(status, indent=2))
    if failure == "timeout":
        return 2
    return 0 if result["ok"] else 1


def store_reply(event, op):
    """The `store` reply for `op` that an event carries, or None."""
    kind, value = event
    if kind in ("reply", "late") and value.get("cmd") == "store" and value.get("op") == op:
        return value
    return None


def request_store_put(console, path, data, timeout):
    """Write `data` to `path` on the deck. Returns (result, failure), failure None, "check", "device" or "timeout".

    The deck keeps the whole file in memory, answers after every `ack_lines` lines with how many bytes it holds, and
    swaps the file in atomically after its last line, so the host never sends more than one window ahead.
    """
    crc = zlib.crc32(data)
    result = {"ok": False, "op": "put", "path": path, "bytes": len(data), "crc32": crc, "lines": 0,
              "seconds": None, "bytes_per_s": None}
    start = time.monotonic()

    def wait(phase):
        deadline = time.monotonic() + timeout
        while True:
            event = console.next_event(deadline)
            if event is None:
                return None, "timeout", f"no {phase!r} reply to the put within {timeout}s"
            reply = store_reply(event, "put")
            if reply is None:
                continue
            if reply.get("ok") is not True:
                return None, "device", reply.get("error") or "firmware reported ok false"
            if reply.get("phase") == phase:
                return reply, None, None

    console.send(f"store put {path} {len(data)} {crc}")
    ready, failure, error = wait("ready")
    if ready is None:
        result["error"] = error
        return result, failure
    line_bytes, ack_lines = ready.get("line_bytes"), ready.get("ack_lines")
    if not (isinstance(line_bytes, int) and 0 < line_bytes <= STORE_LINE_RAW and isinstance(ack_lines, int)
            and ack_lines > 0):
        result["error"] = f"ready reply has no usable line_bytes and ack_lines: {ready}"
        return result, "check"
    chunks = [data[i:i + line_bytes] for i in range(0, len(data), line_bytes)]
    result["lines"] = len(chunks)
    for i, chunk in enumerate(chunks, 1):
        console.send("@tds " + base64.b64encode(chunk).decode("ascii"), note=False)
        if i % ack_lines == 0 and i != len(chunks):
            ack, failure, error = wait("ack")
            if ack is None:
                result["error"] = f"after line {i} of {len(chunks)}: {error}"
                return result, failure
            if ack.get("got") != i * line_bytes:
                result["error"] = f"after line {i} the deck holds {ack.get('got')} bytes, expected {i * line_bytes}"
                return result, "check"
    done, failure, error = wait("done")
    seconds = time.monotonic() - start
    if done is None:
        result["error"] = f"after the last line: {error}"
        return result, failure
    problems = []
    if done.get("crc32") != crc:
        problems.append(f"deck wrote crc32 {done.get('crc32')}, sent {crc}")
    if done.get("bytes") != len(data):
        problems.append(f"deck wrote {done.get('bytes')} bytes, sent {len(data)}")
    result.update(ok=not problems, seconds=round(seconds, 3), bytes_per_s=round(len(data) / max(seconds, 1e-6)))
    if problems:
        result["error"] = "; ".join(problems)
        return result, "check"
    return result, None


def check_store_stream(result, header, trailer, chunks, seen, problems):
    """The checks a `store get` stream must pass, shared by the live reader and the log decoder."""
    raw = b"".join(chunks)
    crc = zlib.crc32(raw)
    size, lines = header.get("bytes"), header.get("lines")
    if not (isinstance(size, int) and isinstance(lines, int)):
        problems.append("start reply lacks integer bytes and lines")
    else:
        if len(raw) != size:
            problems.append(f"got {len(raw)} of {size} bytes")
        if seen != lines:
            problems.append(f"got {seen} of {lines} lines")
    if trailer.get("bytes") != len(raw):
        problems.append(f"end reply says {trailer.get('bytes')} bytes, the lines held {len(raw)}")
    crc_ok = crc == trailer.get("crc32")
    if not crc_ok:
        problems.append(f"crc32 of data is {crc}, end reply says {trailer.get('crc32')}")
    result.update(ok=not problems, bytes=len(raw), lines=seen, crc32=crc, crc_ok=crc_ok)
    if problems:
        result["error"] = "; ".join(problems)
    return raw


def request_store_get(console, path, timeout):
    """Read `path` from the deck. Returns (result, raw, failure), failure None, "check", "device" or "timeout"."""
    result = {"ok": False, "op": "get", "path": path, "bytes": None, "lines": None, "crc32": None, "crc_ok": None,
              "seconds": None, "bytes_per_s": None}
    console.send(f"store get {path}")
    start = time.monotonic()
    deadline = start + timeout
    header = trailer = None
    chunks, problems, seen = [], [], 0
    while trailer is None:
        event = console.next_event(deadline)
        if event is None:
            result["error"] = (f"no store reply within {timeout}s" if header is None
                               else f"stream stalled after {seen} of {header.get('lines')} lines")
            return result, None, "timeout"
        kind, value = event
        if kind == "tds":
            if header is None:
                continue
            seen += 1
            try:
                chunks.append(base64.b64decode(value, validate=True))
            except ValueError:
                problems.append(f"line {seen} is not valid base64")
            continue
        reply = store_reply(event, "get")
        if reply is None:
            continue
        if header is None and reply.get("phase") == "start" and reply.get("ok") is True:
            header = reply
        elif header is None or reply.get("phase") != "end":
            result["error"] = reply.get("error") or "firmware reported ok false"
            return result, None, "device"
        else:
            trailer = reply
            if reply.get("ok") is not True:
                problems.append(reply.get("error") or "the stream ended with ok false")
    seconds = time.monotonic() - start
    raw = check_store_stream(result, header, trailer, chunks, seen, problems)
    result.update(seconds=round(seconds, 3), bytes_per_s=round(len(raw) / max(seconds, 1e-6)))
    return result, (raw if result["ok"] else None), (None if result["ok"] else "check")


def decode_store_log(path):
    """The last complete `store get` stream in a capture's serial.log. Returns (result, raw or None)."""
    result = {"ok": False, "op": "decode", "log": str(path)}
    header = trailer = None
    chunks, seen, problems = [], 0, []
    for line in Path(path).read_text(encoding="utf-8", errors="replace").splitlines():
        match = REPLY_RE.match(line)
        reply = None
        if match:
            try:
                reply = json.loads(match.group(1))
            except ValueError:
                reply = None
        if reply and reply.get("cmd") == "store" and reply.get("op") == "get":
            if reply.get("phase") == "start" and reply.get("ok") is True:
                header, trailer, chunks, seen, problems = reply, None, [], 0, []
            elif reply.get("phase") == "end" and header is not None and trailer is None:
                trailer = reply
                if reply.get("ok") is not True:
                    problems.append(reply.get("error") or "the stream ended with ok false")
            continue
        match = TDS_LOG_RE.search(line)
        if match and header is not None and trailer is None:
            seen += 1
            try:
                chunks.append(base64.b64decode(match.group(1), validate=True))
            except ValueError:
                problems.append(f"line {seen} is not valid base64")
    if header is None or trailer is None:
        result["error"] = "no complete store get stream in the log"
        return result, None
    raw = check_store_stream(result, header, trailer, chunks, seen, problems)
    return result, (raw if result["ok"] else None)


def cmd_store(args):
    if args.store_op == "decode":
        result, raw = decode_store_log(args.log_file)
        if raw is not None and args.out:
            Path(args.out).parent.mkdir(parents=True, exist_ok=True)
            Path(args.out).write_bytes(raw)
            result["out"] = str(args.out)
        print(json.dumps(result, indent=2))
        return 0 if result["ok"] else 1
    data = None
    if args.store_op == "put":
        try:
            data = Path(args.source).read_bytes()
        except OSError as e:
            log(f"error: cannot read {args.source}: {e}")
            return 2
        if not 1 <= len(data) <= STORE_PUT_MAX:
            log(f"error: {args.source} is {len(data)} bytes, a put takes 1 to {STORE_PUT_MAX}")
            return 2
    try:
        device = resolve_port(args.port)
    except SystemExit as e:
        log(str(e))
        return 2
    try:
        ser = open_port(device)
    except serial.SerialException as e:
        log(f"error: cannot open {device}: {e}")
        return 2
    ser.write_timeout = args.timeout
    transcript = None
    if args.log:
        Path(args.log).parent.mkdir(parents=True, exist_ok=True)
        transcript = open(args.log, "w", encoding="utf-8")
    console = Console(ser, transcript)
    raw = None
    try:
        ser.reset_input_buffer()
        if args.store_op == "put":
            result, failure = request_store_put(console, args.path, data, args.timeout)
        else:
            result, raw, failure = request_store_get(console, args.path, args.timeout)
    except serial.SerialTimeoutException:
        log(f"error: {device} did not accept the line within {args.timeout}s (firmware without a store?)")
        return 2
    except serial.SerialException as e:
        log(f"error: lost {device}: {e}")
        return 2
    finally:
        if transcript:
            transcript.close()
        try:
            ser.close()
        except serial.SerialException:
            pass
    result["port"] = device
    if raw is not None:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_bytes(raw)
        result["out"] = str(out)
    if failure:
        log(f"error: store {args.store_op} {args.path}: {result['error']}")
    else:
        log(f"store {args.store_op} {args.path}: {result['bytes']} bytes in {result['seconds']}s "
            f"({result['bytes_per_s'] // 1024} KiB/s), crc32 {result['crc32']} ok")
    print(json.dumps(result, indent=2))
    if failure == "timeout":
        return 2
    return 0 if result["ok"] else 1


def cmd_unit(args):
    device = resolve_port(args.port)
    wait_for_port(device, timeout=10)
    out_dir = Path(args.out) if args.out else Path(tempfile.mkdtemp(prefix="touchdeck-unit-"))
    out_dir.mkdir(parents=True, exist_ok=True)
    transcript = out_dir / "serial.log"
    matches, lines = record(device, args.seconds, [UNITY_SUMMARY_RE, UNITY_RESULT_RE], transcript,
                            reset=True, stop_when_matched=True)
    summary_line = matches[0]["line"]
    report = {"port": device, "tests": None, "failures": None, "ignored": None, "result": None,
              "failed": [line for line in lines if ":FAIL" in line], "ok": False,
              "transcript": str(transcript)}
    if summary_line:
        tests, failures, ignored = map(int, re.search(UNITY_SUMMARY_RE, summary_line).groups())
        report.update(tests=tests, failures=failures, ignored=ignored)
    if matches[1]["line"]:
        report["result"] = matches[1]["line"].strip()
    report["ok"] = bool(report["tests"]) and report["failures"] == 0 and report["result"] == "OK"
    print(json.dumps(report, indent=2))
    if summary_line is None:
        return 2
    return 0 if report["ok"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("ports", help="list Espressif USB devices")
    p.set_defaults(func=cmd_ports)

    p = sub.add_parser("doctor", help="is this board worth driving?")
    p.add_argument("--port", help="serial port (default: the single Espressif USB-Serial/JTAG port)")
    p.add_argument("--seconds", type=float, default=10, help="max time to wait for banner + heartbeat")
    p.set_defaults(func=cmd_doctor)

    p = sub.add_parser("capture", help="record serial output and check expectations")
    p.add_argument("--port", help="serial port (default: the single Espressif USB-Serial/JTAG port)")
    p.add_argument("--seconds", type=float, default=10, help="how long to record")
    p.add_argument("--out", required=True, help="evidence directory (gets serial.log and summary.json)")
    p.add_argument("--expect", action="append", help="regex that must appear, in order; repeatable")
    p.add_argument("--no-reset", action="store_true", help="attach without resetting the board")
    p.add_argument("--stop-when-matched", action="store_true", help="stop as soon as every --expect matched")
    p.add_argument("--send", action="append", help="console line to write once during the capture; repeatable")
    p.add_argument("--send-hex", dest="send", action="append", type=bytes.fromhex,
                   help="raw bytes to write in order with --send, without an added newline")
    p.add_argument("--send-after", type=float, default=1.0, help="seconds after the capture starts to write the --send lines")
    p.add_argument("--send-times", type=int, default=1, help="write the --send lines this many times in total")
    p.add_argument("--send-every", type=float, default=0.0, help="seconds between repeats of the --send lines")
    p.set_defaults(func=cmd_capture)

    p = sub.add_parser("cmd", help="send one console line and print its @td JSON reply")
    p.add_argument("line", help='console line, e.g. "version"')
    p.add_argument("--port", help="serial port (default: the single Espressif USB-Serial/JTAG port)")
    p.add_argument("--timeout", type=float, default=3, help="seconds to wait for each reply")
    p.add_argument("--repeat", type=int, default=1, help="send the line this many times in one session")
    p.add_argument("--interval", type=float, default=0, help="seconds to wait between repeats")
    p.add_argument("--time", action="store_true", help="print each round trip in ms to stderr")
    p.set_defaults(func=cmd_cmd)

    p = sub.add_parser("screenshot", help="stream the display over the console and save it as a PNG")
    p.add_argument("--port", help="serial port (default: the single Espressif USB-Serial/JTAG port)")
    p.add_argument("--scale", type=int, choices=(1, 2, 4), default=1, help="keep every Nth pixel in both axes")
    p.add_argument("--out", help="PNG file, required unless --raw is given; with --count N the files are base-1.png ... base-N.png")
    p.add_argument("--raw", help="also write the raw little-endian RGB565 bytes to this file")
    p.add_argument("--count", type=int, default=1, help="screenshots to take in one serial session")
    p.add_argument("--background", help='console line to send once before the first screenshot without waiting for its reply, e.g. "perf fps 10"')
    p.add_argument("--background-timeout", type=float, default=30, help="seconds after sending it to wait for its reply")
    p.add_argument("--log", help="write every non-image line the console printed, with time offsets, to this file")
    p.add_argument("--timeout", type=float, default=20, help="seconds to wait for each screenshot")
    p.add_argument("--retries", type=int, default=0, help="times to request a screenshot again after a CRC or length failure")
    p.set_defaults(func=cmd_screenshot)

    p = sub.add_parser("keys", help="read the key engine's record over the console and save it as JSON")
    p.add_argument("--port", help="serial port (default: the single Espressif USB-Serial/JTAG port)")
    p.add_argument("--out", required=True, help="JSON file for {count, dropped, reports, brief}")
    p.add_argument("--log", help="write every non-record line the console printed, with time offsets, to this file")
    p.add_argument("--timeout", type=float, default=30, help="seconds to wait for the whole record")
    p.set_defaults(func=cmd_keys)

    p = sub.add_parser("store", help="put a file on the deck's storage partition or get one back, checking its CRC-32")
    ops = p.add_subparsers(dest="store_op", required=True)
    for name, text in (("put", "write a local file to a /storage path atomically"),
                       ("get", "read a /storage path into a local file")):
        q = ops.add_parser(name, help=text)
        q.add_argument("path", help="path on the deck, such as /storage/test.bin")
        if name == "put":
            q.add_argument("--from", dest="source", required=True, help="local file to send, 1 to 65536 bytes")
        else:
            q.add_argument("--out", required=True, help="local file to write; nothing is written when a check fails")
        q.add_argument("--port", help="serial port (default: the single Espressif USB-Serial/JTAG port)")
        q.add_argument("--timeout", type=float, default=30, help="seconds to wait for each reply, and for a get's lines")
        q.add_argument("--log", help="write every non-stream line the console printed, with time offsets, to this file")
    q = ops.add_parser("decode", help="decode the last store get stream in a capture's serial.log")
    q.add_argument("log_file", help="serial.log of a capture that sent store get")
    q.add_argument("--out", help="local file to write when every check passes")
    p.set_defaults(func=cmd_store)

    p = sub.add_parser("unit", help="reset the board and report the Unity summary of firmware/test_app")
    p.add_argument("--port", help="serial port (default: the single Espressif USB-Serial/JTAG port)")
    p.add_argument("--seconds", type=float, default=30, help="max time to wait for the Unity summary")
    p.add_argument("--out", help="evidence directory (gets serial.log); default is a temp dir")
    p.set_defaults(func=cmd_unit)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
