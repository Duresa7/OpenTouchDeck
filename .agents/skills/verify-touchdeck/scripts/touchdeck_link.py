"""SLIP app link on the existing USB-Serial/JTAG console."""

import argparse
import hashlib
import json
import os
import re
import struct
import sys
import time
from pathlib import Path

END = 0xC0
ESC = 0xDB
PAYLOAD_MAX = 1024
BODY_MAX = PAYLOAD_MAX + 8
WIRE_MAX = 2 * BODY_MAX + 2
CHUNK_HEADER = 3
CHUNK_DATA_MAX = PAYLOAD_MAX - CHUNK_HEADER


def crc16(data):
    crc = 0xFFFF
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ (0x1021 if crc & 0x8000 else 0)) & 0xFFFF
    return crc


def check_header(kind, seq, flags, length):
    if length > PAYLOAD_MAX:
        raise ValueError("length")
    if (any(type(value) is not int for value in (kind, seq, flags))
            or kind not in (1, 2, 3) or not 0 <= seq <= 255 or flags not in (0, 1, 2)
            or (flags == 2 and seq != 0) or (kind == 2 and length < CHUNK_HEADER)
            or (kind == 3 and length != 3)):
        raise ValueError("header")


def encode_frame(kind, seq, flags, payload):
    check_header(kind, seq, flags, len(payload))
    body = struct.pack("<BBBBH", 1, kind, seq, flags, len(payload)) + payload
    body += struct.pack("<H", crc16(body))
    return bytes([END]) + body.replace(b"\xdb", b"\xdb\xdd").replace(b"\xc0", b"\xdb\xdc") + bytes([END])


def decode_body(body):
    if len(body) < 8 or len(body) > BODY_MAX:
        raise ValueError("length")
    ver, kind, seq, flags, length = struct.unpack_from("<BBBBH", body)
    if ver != 1:
        raise ValueError("version")
    if length > PAYLOAD_MAX or len(body) != length + 8:
        raise ValueError("length")
    if crc16(body[:-2]) != struct.unpack_from("<H", body, len(body) - 2)[0]:
        raise ValueError("crc")
    payload = bytes(body[6:-2])
    check_header(kind, seq, flags, len(payload))
    return {"ver": ver, "type": kind, "seq": seq, "flags": flags,
            "len": length, "payload_hex": payload.hex()}


def decode_frame(wire):
    if len(wire) < 2 or wire[0] != END or wire[-1] != END:
        raise ValueError("length")
    body = bytearray()
    escaped = False
    for byte in wire[1:-1]:
        if byte == END:
            raise ValueError("escape")
        if escaped:
            if byte not in (0xDC, 0xDD):
                raise ValueError("escape")
            body.append(END if byte == 0xDC else ESC)
            escaped = False
        elif byte == ESC:
            escaped = True
        else:
            body.append(byte)
    if escaped:
        raise ValueError("escape")
    return decode_body(body)


FRAME_DEADLINE_S = 0.5


class StreamDecoder:
    """Splits frames from text by the deck's rules in link-v1.md. Bytes that prove not to be a frame come back as text.

    Feed it on idle reads too, with empty data, so a frame left open past its deadline gives its bytes back.
    """

    def __init__(self, clock=time.monotonic):
        self.clock = clock
        self.mode = "text"
        self.body = bytearray()
        self.raw = bytearray()
        self.limit = BODY_MAX
        self.opened = 0.0
        self.errors = 0

    def give_back(self, text):
        if self.mode in ("frame", "escape"):
            self.errors += 1
            text.extend(self.raw)
        self.mode = "text"

    def end_frame(self, text):
        if self.mode == "frame":
            try:
                return decode_body(self.body)
            except ValueError:
                pass
        self.give_back(text)
        return None

    def take(self, byte, text):
        self.raw.append(byte)
        if self.mode == "escape":
            if byte not in (0xDC, 0xDD):
                self.give_back(text)
                return
            byte = END if byte == 0xDC else ESC
        elif byte == ESC:
            self.mode = "escape"
            return
        self.mode = "frame"
        self.body.append(byte)
        if len(self.body) == 6:
            self.limit = 8 + (self.body[4] | self.body[5] << 8)
            try:
                check_header(self.body[1], self.body[2], self.body[3], self.limit - 8)
            except ValueError:
                self.give_back(text)
        elif len(self.body) > self.limit:
            self.give_back(text)

    def feed_events(self, data):
        text = bytearray()
        events = []
        now = self.clock()
        if self.mode != "text" and now - self.opened >= FRAME_DEADLINE_S:
            self.give_back(text)
        for byte in data:
            if byte == END:
                frame = self.end_frame(text)
                if text:
                    events.append(("text", bytes(text)))
                    text.clear()
                if frame is not None:
                    events.append(("frame", frame))
                self.mode = "open"
                self.opened = now
                self.body.clear()
                self.raw.clear()
                self.limit = BODY_MAX
            elif self.mode == "text":
                text.append(byte)
            elif self.mode == "open" and byte != 1:
                self.mode = "text"
                text.append(byte)
            else:
                self.take(byte, text)
        if text:
            events.append(("text", bytes(text)))
        return events

    def feed(self, data):
        events = self.feed_events(data)
        return (b"".join(value for kind, value in events if kind == "text"),
                [value for kind, value in events if kind == "frame"])


def reject_constant(token):
    raise ValueError(token)


def message(frame):
    if frame["type"] != 1:
        raise ValueError("expected a message frame")
    value = json.loads(bytes.fromhex(frame["payload_hex"]).decode("utf-8"),
                       parse_constant=reject_constant)
    if not isinstance(value, dict) or not isinstance(value.get("m"), str):
        raise ValueError("expected an object with m")
    return value


def validate_reply(frame, seq=None, request=None):
    if frame["flags"] != 1 or (seq is not None and frame["seq"] != seq):
        raise ValueError("response sequence or flags")
    reply = message(frame)
    if not isinstance(reply.get("ok"), bool):
        raise ValueError("missing response status")
    if request is None:
        return reply
    if reply["m"] != request["m"]:
        raise ValueError("response message")
    if not reply["ok"]:
        return reply
    if request["m"] == "hello":
        if (reply.get("product") != "touchdeck" or reply.get("proto") != 1
                or not isinstance(reply.get("elf_sha256"), str)
                or len(reply["elf_sha256"]) != 64
                or any(c not in "0123456789abcdef" for c in reply["elf_sha256"])
                or not isinstance(reply.get("version"), str)
                or not isinstance(reply.get("running"), str)
                or reply.get("sinks") not in (["log"], ["log", "link"])
                or not isinstance(reply.get("recovery"), bool)
                or reply.get("layout") not in ("builtin", "storage")):
            raise ValueError("hello fields")
    elif request["m"] == "ping":
        if reply.get("n") != request.get("n") or type(reply.get("t")) is not int or reply["t"] < 0:
            raise ValueError("ping fields")
    elif request["m"] == "link.state":
        if not isinstance(reply.get("online"), bool):
            raise ValueError("state online")
        for key in ("frames_rx", "frames_tx", "crc_errors", "dropped", "last_rx_ms"):
            if type(reply.get(key)) is not int or reply[key] < 0:
                raise ValueError(f"state {key}")
    return reply


def exchange(ser, decoder, wire, timeout, seq=None, request=None, transcript=None):
    start = time.monotonic()
    if ser.write(wire) != len(wire):
        raise ValueError("short write")
    ser.flush()
    deadline = start + timeout
    while time.monotonic() < deadline:
        text, frames = decoder.feed(ser.read(ser.in_waiting or 1))
        if transcript and text:
            transcript.write(text.decode("utf-8", errors="replace"))
            transcript.flush()
        for frame in frames:
            if frame["flags"] != 1 or (seq is not None and frame["seq"] != seq):
                continue
            return validate_reply(frame, seq, request), (time.monotonic() - start) * 1000
    raise TimeoutError(f"no reply within {timeout:g}s")


def scenario_actions(document):
    from touchdeck_serial import console_word
    if not isinstance(document, dict) or not isinstance(document.get("steps"), list):
        raise ValueError("a scenario needs a steps array")
    for key, default, limit in (("duration_ms", 10000, 600000), ("timeout_ms", 500, 600000),
                                ("window", 1, 255)):
        value = document.get(key, default)
        if type(value) is not int or not 1 <= value <= limit:
            raise ValueError(f"invalid {key}")
    actions, groups = [], set()
    for index, step in enumerate(document["steps"]):
        if not isinstance(step, dict):
            raise ValueError("each step must be an object")
        name = step.get("id", str(index))
        if not isinstance(name, str) or name in groups:
            raise ValueError("step ids must be unique strings")
        groups.add(name)
        kinds = [key for key in ("text", "hex", "message", "frame") if key in step]
        if len(kinds) != 1:
            raise ValueError("each step needs exactly one of text, hex, message, frame")
        kind = kinds[0]
        count = step.get("count", 1)
        at = step.get("at_ms", 0)
        every = step.get("every_ms", 0)
        if (type(count) is not int or not 1 <= count <= 10000
                or type(at) is not int or at < 0 or type(every) is not int or every < 0):
            raise ValueError("invalid step count or time")
        after = step.get("after", [])
        after = [after] if isinstance(after, str) else after
        if not isinstance(after, list) or any(not isinstance(item, str) for item in after):
            raise ValueError("after must name step ids")
        for repetition in range(count):
            action = {"id": f"{name}[{repetition}]", "group": name, "after": after,
                      "at_ms": at + repetition * every, "kind": kind, "request": None,
                      "expect": step.get("expect"), "max": step.get("max", {}),
                      "expect_reply": step.get("expect_reply", True)}
            if type(action["expect_reply"]) is not bool or not isinstance(action["max"], dict):
                raise ValueError("invalid reply expectation")
            if kind == "text":
                line = step[kind]
                if not isinstance(line, str) or "\n" in line or "\r" in line:
                    raise ValueError("text must be one line without CR or LF")
                word = console_word(line)
                action["wire"] = line.encode("utf-8") + b"\n"
                if action["expect"] is None:
                    action["expect"] = {"cmd": word[:63] if word else "", "ok": True}
                if not word and action["expect_reply"]:
                    raise ValueError("an empty console line has no reply")
            elif kind == "message":
                request = step[kind]
                if not isinstance(request, dict) or not isinstance(request.get("m"), str):
                    raise ValueError("message must be an object with m")
                request = dict(request)
                if count > 1 and request["m"] == "ping":
                    if type(request.get("n", 0)) is not int:
                        raise ValueError("repeated ping needs integer n")
                    request["n"] = request.get("n", 0) + repetition
                action["request"] = request
                action["payload"] = json.dumps(request, separators=(",", ":"), allow_nan=False).encode()
                encode_frame(1, 1, 0, action["payload"])
            else:
                if kind == "hex":
                    action["wire"] = bytes.fromhex(step[kind])
                else:
                    frame = step[kind]
                    action["wire"] = encode_frame(frame["type"], frame["seq"], frame.get("flags", 0),
                                                  bytes.fromhex(frame["payload_hex"]))
                if action["expect_reply"]:
                    frame = decode_frame(action["wire"])
                    if frame["flags"] != 0:
                        raise ValueError("only a request can expect a response")
                    action["seq"] = frame["seq"]
                    if frame["type"] == 1:
                        action["request"] = message(frame)
            if action["expect"] is not None and not isinstance(action["expect"], dict):
                raise ValueError("expect must be an object")
            actions.append(action)
    if len(actions) > 10000 or any(set(a["after"]) - groups for a in actions):
        raise ValueError("too many sends or unknown dependency")
    return sorted(actions, key=lambda action: action["at_ms"])


def run_scenario(ser, document, clock=time.monotonic):
    from touchdeck_serial import REPLY_RE, TDR_RE, brief_report
    actions = scenario_actions(document)
    result = {"ok": False, "sent": [], "console": [], "logs": [], "frames": [],
              "events": [], "records": [], "errors": []}
    pending_text, pending_frames = [], {}
    completed, group_counts = set(), {}
    for action in actions:
        group_counts[action["group"]] = group_counts.get(action["group"], 0) + 1
    decoder, tail = StreamDecoder(), b""
    start = clock()
    seq = 0

    def finish(action, reply, elapsed):
        expected = action["expect"] or {"ok": True}
        if reply is not None and any(reply.get(key) != value for key, value in expected.items()):
            result["errors"].append(f"{action['id']}: reply did not match {expected}")
        for key, limit in action["max"].items():
            value = None if reply is None else reply.get(key)
            if type(value) not in (int, float) or value > limit:
                result["errors"].append(f"{action['id']}: {key} exceeded {limit} or was absent")
        action["receipt"]["rtt_ms"] = round(elapsed - action["sent_ms"], 3)
        group_counts[action["group"]] -= 1
        if group_counts[action["group"]] == 0:
            completed.add(action["group"])

    while (clock() - start) * 1000 < document.get("duration_ms", 10000):
        elapsed = (clock() - start) * 1000
        for action in actions:
            if "sent_ms" in action or action["at_ms"] > elapsed or not set(action["after"]) <= completed:
                continue
            is_frame = action["kind"] != "text"
            if is_frame and action["expect_reply"]:
                if len(pending_frames) >= document.get("window", 1):
                    continue
                if action["kind"] == "message":
                    for _ in range(255):
                        seq = seq % 255 + 1
                        if seq not in pending_frames:
                            break
                    action["seq"] = seq
                    action["wire"] = encode_frame(1, seq, 0, action["payload"])
                elif action["seq"] in pending_frames:
                    continue
            elif action["kind"] == "message":
                action["wire"] = encode_frame(1, 0, 0, action["payload"])
            wire = action["wire"]
            if ser.write(wire) != len(wire):
                raise ValueError("short scenario write")
            action["sent_ms"] = (clock() - start) * 1000
            receipt = {"id": action["id"], "scheduled_ms": action["at_ms"],
                       "t_ms": round(action["sent_ms"], 3), "kind": action["kind"]}
            if "seq" in action:
                receipt["seq"] = action["seq"]
            action["receipt"] = receipt
            result["sent"].append(receipt)
            if action["expect_reply"]:
                if is_frame:
                    pending_frames[action["seq"]] = action
                else:
                    pending_text.append(action)
            else:
                group_counts[action["group"]] -= 1
                if group_counts[action["group"]] == 0:
                    completed.add(action["group"])
        chunk = ser.read(ser.in_waiting or 1)
        elapsed = (clock() - start) * 1000
        for kind, value in decoder.feed_events(chunk):
            stamp = {"t_ms": round(elapsed, 3)}
            if kind == "text":
                *lines, tail = (tail + value).split(b"\n")
                for raw in lines:
                    line = raw.decode("utf-8", errors="replace").rstrip("\r")
                    matched = REPLY_RE.match(line)
                    record = TDR_RE.match(raw.rstrip(b"\r"))
                    found = record or matched
                    try:
                        parsed = json.loads(found.group(1)) if found else None
                    except ValueError as error:
                        result["errors"].append(f"unreadable console line {line!r}: {error}")
                        continue
                    if record:
                        result["records"].append(parsed)
                    elif matched:
                        reply = parsed
                        entry = dict(stamp, reply=reply)
                        result["console"].append(entry)
                        candidates = [a for a in pending_text if a["expect"].get("cmd") == reply.get("cmd")]
                        if candidates:
                            action = candidates[0]
                            phase = action["expect"].get("phase")
                            if phase is not None and reply.get("phase") != phase and reply.get("ok"):
                                continue
                            if pending_text[0] is not action:
                                result["errors"].append("console reply order")
                            pending_text.remove(action)
                            entry["id"] = action["id"]
                            finish(action, reply, elapsed)
                    else:
                        result["logs"].append(dict(stamp, line=line))
            else:
                entry = dict(stamp, frame=value)
                if value["type"] == 1:
                    try:
                        entry["message"] = message(value)
                    except ValueError as error:
                        result["errors"].append(f"bad frame seq {value['seq']}: {error}")
                if value["flags"] == 2:
                    result["events"].append(entry)
                    continue
                result["frames"].append(entry)
                action = pending_frames.get(value["seq"]) if value["flags"] == 1 else None
                if action is None:
                    result["errors"].append(f"unmatched frame seq {value['seq']}")
                    continue
                if next(iter(pending_frames)) != value["seq"]:
                    result["errors"].append("link reply order")
                del pending_frames[value["seq"]]
                entry["id"] = action["id"]
                try:
                    reply = validate_reply(value, value["seq"], action["request"]) if value["type"] == 1 else value
                except ValueError as error:
                    result["errors"].append(f"{action['id']}: bad reply: {error}")
                    reply = None
                finish(action, reply, elapsed)
        for action in list(pending_text) + list(pending_frames.values()):
            if elapsed - action["sent_ms"] >= document.get("timeout_ms", 500):
                result["errors"].append(f"{action['id']}: reply timeout")
                if action in pending_text:
                    pending_text.remove(action)
                else:
                    del pending_frames[action["seq"]]
    for action in actions:
        if "sent_ms" not in action:
            result["errors"].append(f"{action['id']}: not sent before scenario ended")
    for action in pending_text + list(pending_frames.values()):
        result["errors"].append(f"{action['id']}: no reply before scenario ended")
    result["crc_errors"] = decoder.errors
    if decoder.errors:
        result["errors"].append("malformed received frame")
    result["heartbeat_ms"] = [int(match.group(1)) for entry in result["logs"]
                              if (match := re.search(r"\((\d+)\) touchdeck: alive \d+", entry["line"]))]
    result["heartbeat_gaps_ms"] = [b - a for a, b in zip(result["heartbeat_ms"], result["heartbeat_ms"][1:])]
    if "heartbeat_gap_ms" in document:
        low, high = document["heartbeat_gap_ms"]
        if (len(result["heartbeat_ms"]) < document.get("heartbeat_min", 3)
                or any(not low <= gap <= high for gap in result["heartbeat_gaps_ms"])):
            result["errors"].append("heartbeat count or gap")
    result["record_brief"] = [brief_report(record) for record in result["records"]]
    if "record_brief" in document and result["record_brief"] != document["record_brief"]:
        result["errors"].append("key record mismatch")
    result["ok"] = not result["errors"]
    return result


def scenario_selftest():
    class Port:
        def __init__(self, frame_first=False, wrong_seq=False, silent=False, bad_first=False):
            self.now = 0
            self.output = bytearray(b"I (2000) touchdeck: alive 1\nI (4000) touchdeck: alive 2\n"
                                    b"I (6000) touchdeck: alive 3\n")
            self.held = b""
            self.frame_first = frame_first
            self.wrong_seq = wrong_seq
            self.silent = silent
            self.bad_first = bad_first
            self.writes = []

        @property
        def in_waiting(self):
            return len(self.output)

        def read(self, size):
            self.now += 0.001
            count = min(size, 7)
            data = bytes(self.output[:count])
            del self.output[:count]
            return data

        def write(self, wire):
            self.writes.append((self.now, wire))
            if self.silent:
                return len(wire)
            if wire[0] == END:
                frame = decode_frame(wire)
                request = message(frame)
                reply = json.dumps(dict(request, ok=True, t=1234)).encode()
                if self.bad_first:
                    reply, self.bad_first = b"not json", False
                seq = (frame["seq"] + int(self.wrong_seq)) % 256
                self.output.extend(encode_frame(1, seq, 1, reply))
                self.output.extend(encode_frame(1, 0, 2, b'{"m":"test.event"}'))
                self.output.extend(self.held)
                self.held = b""
            else:
                word = wire.decode().strip().split(" ")[0][:63]
                reply = {"cmd": word, "ok": True}
                if len(wire) > 256:
                    reply.update(ok=False, error="line too long")
                data = b"td> @td " + json.dumps(reply).encode() + b"\r\n"
                if self.frame_first and len(wire) > 256:
                    self.held += data
                else:
                    self.output.extend(data)
            return len(wire)

    mixed = {"duration_ms": 200, "timeout_ms": 100, "heartbeat_gap_ms": [1800, 2800], "steps": [
        {"text": "x" * 300, "expect": {"cmd": "x" * 63, "ok": False, "error": "line too long"}},
        {"hex": encode_frame(1, 23, 0, b'{"m":"ping","n":7}').hex()},
        {"text": "version", "at_ms": 80},
    ]}
    for frame_first in (False, True):
        port = Port(frame_first=frame_first)
        result = run_scenario(port, mixed, lambda: port.now)
        assert result["ok"], result["errors"]
        assert [entry["reply"]["cmd"] for entry in result["console"]] == ["x" * 63, "version"]
        assert result["frames"][0]["frame"]["seq"] == 23
        assert len(result["events"]) == 1
        assert port.writes[-1][0] >= 0.080
        first_text = result["console"][0]["t_ms"]
        first_frame = result["frames"][0]["t_ms"]
        assert (first_frame < first_text) == frame_first
    flood = {"duration_ms": 10000, "timeout_ms": 500, "window": 4, "steps": [
        {"id": "pings", "message": {"m": "ping", "n": 0}, "count": 300},
        {"text": "version", "after": "pings"},
    ]}
    port = Port()
    result = run_scenario(port, flood, lambda: port.now)
    assert result["ok"], result["errors"]
    assert [item["message"]["n"] for item in result["frames"]] == list(range(300))
    assert result["sent"][254]["seq"] == 255 and result["sent"][255]["seq"] == 1
    assert len(result["events"]) == 300
    assert result["sent"][-1]["kind"] == "text"
    single = {"duration_ms": 100, "timeout_ms": 50, "steps": [{"message": {"m": "ping", "n": 0}}]}
    for port in (Port(wrong_seq=True), Port(silent=True)):
        result = run_scenario(port, single, lambda: port.now)
        assert not result["ok"] and any("timeout" in error for error in result["errors"])
    port = Port(bad_first=True)
    pair = {"duration_ms": 100, "timeout_ms": 50, "steps": [{"message": {"m": "ping", "n": 0}, "count": 2}]}
    result = run_scenario(port, pair, lambda: port.now)
    assert not result["ok"] and any("bad reply" in error for error in result["errors"]), result["errors"]
    assert [entry["message"]["n"] for entry in result["frames"] if "message" in entry] == [1]
    return {"mixed_channel_orders": 2, "pings": 300, "sequence_wrap": True, "events": 300,
            "negative_cases": ["wrong_sequence", "timeout", "bad_reply_then_good_reply"]}


def selftest(path):
    document = json.loads(path.read_text(encoding="utf-8"))
    assert document["protocol"] == 1
    assert crc16(b"123456789") == 0x29B1
    assert BODY_MAX == 1032 and WIRE_MAX == 2066 and CHUNK_DATA_MAX == 1021
    fixtures = document["frames"]
    assert len(fixtures) >= 12
    hello = next(bytes.fromhex(frame["hex"]) for frame in fixtures if frame["valid"])
    valid = 0
    for fixture in fixtures:
        wire = bytes.fromhex(fixture["hex"])
        if fixture["valid"]:
            expected = fixture["decoded"]
            assert decode_frame(wire) == expected, fixture["name"]
            assert encode_frame(expected["type"], expected["seq"], expected["flags"],
                                bytes.fromhex(expected["payload_hex"])) == wire, fixture["name"]
            valid += 1
            for width in (1, 2, 7, 64, 4096):
                decoder = StreamDecoder()
                stream = b"version\r\n" + wire + wire + b"a\xdb\xdd\x00\n"
                text, frames = bytearray(), []
                for start in range(0, len(stream), width):
                    part, received = decoder.feed(stream[start:start + width])
                    text.extend(part)
                    frames.extend(received)
                assert text == b"version\r\na\xdb\xdd\x00\n", fixture["name"]
                assert frames == [expected, expected], fixture["name"]
                assert decoder.errors == 0
        else:
            try:
                decode_frame(wire)
            except ValueError as error:
                assert str(error) == fixture["error"], fixture["name"]
            else:
                raise AssertionError(f"accepted {fixture['name']}")
            decoder = StreamDecoder()
            text, frames = decoder.feed(wire + hello + b"version\n")
            assert text.endswith(b"version\n") and frames == [decode_frame(hello)], fixture["name"]
            assert decoder.errors == (0 if fixture["error"] == "version" else 1), fixture["name"]
    rules = decoder_selftest(hello)
    try:
        encode_frame(2, 1, 0, bytes(1025))
    except ValueError:
        pass
    else:
        raise AssertionError("encoded 1025 bytes")
    maximum = next(frame for frame in fixtures if frame["name"] == "maximum-escaped-chunk")
    payload = bytes.fromhex(maximum["decoded"]["payload_hex"])
    assert len(payload) == PAYLOAD_MAX and set(payload) == {END, ESC}
    assert len(bytes.fromhex(maximum["hex"])) <= WIRE_MAX
    return {"ok": True, "fixtures": str(path), "frames": len(fixtures),
            "valid": valid, "invalid": len(fixtures) - valid, "read_sizes": [1, 2, 7, 64, 4096],
            "decoder_rules": rules, "scenario": scenario_selftest()}


def decoder_selftest(hello):
    expected = decode_frame(hello)
    ping_a = encode_frame(1, 9, 0, b'{"m":"ping","n":0}')
    ping_b = encode_frame(1, 10, 0, b'{"m":"ping","n":1}')
    checks = {
        "first byte not ver opens no frame": (b"\xc0version\n\xc0\xc0heap\n", b"version\nheap\n", [], 0),
        "delimiter after a frame keeps text": (hello + b"\xc0version\n", b"version\n", [expected], 0),
        "closed frame then ver opens the next": (hello + hello[1:] + b"x\n", b"x\n", [expected] * 2, 0),
        "lost closing delimiter costs nothing": (hello[:-1] + hello + b"x\n", b"x\n", [expected] * 2, 0),
        "padded cut frame never hides the next": (ping_a[:18] + b"version\r\n" + ping_b + b"uptime\r\n",
                                                  ping_a[1:18] + b"version\r\nuptime\r\n",
                                                  [decode_frame(ping_b)], 1),
        "invalid header returns at once": (b"x" + hello[:6] + b"I (1) touchdeck: alive 0\n" + hello + b"done\n",
                                           b"x" + hello[1:6] + b"I (1) touchdeck: alive 0\ndone\n", [expected], 1),
        "body past len plus 8 returns at once": (b"\xc0\x01\x01\x01\x00\x02\x00abcdefgh\n",
                                                 b"\x01\x01\x01\x00\x02\x00abcdefgh\n", [], 1),
        "bad escape returns at once": (b"\xc0\x01\xdbxyz\n", b"\x01\xdbxyz\n", [], 1),
    }
    for name, (stream, text_expected, frames_expected, errors) in checks.items():
        for width in (1, 3, 64, 4096):
            decoder = StreamDecoder(clock=lambda: 0.0)
            text, frames = bytearray(), []
            for start in range(0, len(stream), width):
                part, received = decoder.feed(stream[start:start + width])
                text.extend(part)
                frames.extend(received)
            assert (bytes(text), frames, decoder.errors) == (text_expected, frames_expected, errors), name
    now = [0.0]
    decoder = StreamDecoder(clock=lambda: now[0])
    assert decoder.feed(hello[:10]) == (b"", [])
    now[0] = 0.4
    assert decoder.feed(hello[10:14]) == (b"", [])
    now[0] = FRAME_DEADLINE_S - 0.001
    assert decoder.feed(b"") == (b"", []), "idle flush waits for the deadline"
    now[0] = FRAME_DEADLINE_S
    assert decoder.feed(b"") == (hello[1:14], []) and decoder.errors == 1, "idle flush from the opener"
    assert decoder.feed(hello[14:] + b"td> ok\r\n") == (hello[14:-1] + b"td> ok\r\n", []), "tail after deadline"
    now[0] = 10.0
    assert decoder.feed(hello[:5]) == (b"", [])
    now[0] = 10.0 + FRAME_DEADLINE_S - 0.001
    assert decoder.feed(hello[5:]) == (b"", [expected]), "frame inside its deadline"
    return sorted(checks) + ["idle flush from the opener", "frame inside its deadline"]


def keep_online(ser, stop, every=1.0):
    """Write a ping every `every` seconds until `stop` is set, so the deck's link stays online.

    Replies are not awaited. Whatever reads the port meanwhile drops them, because they carry seq 200 to 254 and
    nothing waits on those.
    """
    n = 0
    while not stop.wait(every):
        payload = json.dumps({"m": "ping", "n": n}, separators=(",", ":")).encode()
        ser.write(encode_frame(1, 200 + n % 55, 0, payload))
        n += 1


def online_session(ser, args, messages):
    """Say hello, send each message and check its reply, then hold the link online for --seconds.

    With --shot, take one screenshot --shot-at seconds after hello, on the same port, while the link stays online.
    Returns the JSON result.
    """
    import threading
    from touchdeck_serial import Console, encode_png, take_screenshot
    decoder = StreamDecoder()
    result = {"ok": True, "replies": []}
    started = time.monotonic()
    seq = 0
    for request in [{"m": "hello"}] + messages:
        seq += 1
        wire = encode_frame(1, seq, 0, json.dumps(request, separators=(",", ":")).encode())
        # The hello check in validate_reply predates M2-05's sinks and layout fields, so hello is checked loosely.
        reply, elapsed = exchange(ser, decoder, wire, args.timeout, seq, None if request["m"] == "hello" else request)
        result["replies"].append(reply)
        result["ok"] = result["ok"] and reply.get("ok") is True
    stop = threading.Event()
    pinger = threading.Thread(target=keep_online, args=(ser, stop), daemon=True)
    pinger.start()
    try:
        if args.shot:
            delay = started + args.shot_at - time.monotonic()
            if delay > 0:
                time.sleep(delay)
            console = Console(ser)
            shot, raw, failure = take_screenshot(console, args.scale, 20, 1)
            if raw is not None:
                args.shot.parent.mkdir(parents=True, exist_ok=True)
                args.shot.write_bytes(encode_png(shot["width"], shot["height"], raw))
                shot["out"] = str(args.shot)
            result["shot"] = shot
            result["ok"] = result["ok"] and failure is None
        while time.monotonic() - started < args.seconds:
            ser.read(ser.in_waiting or 1)
    finally:
        stop.set()
        pinger.join()
    result["held_s"] = round(time.monotonic() - started, 3)
    return result


UPDATE_WINDOW = 4
UPDATE_RETRY_S = 0.2
UPDATE_TRANSMISSIONS = 5
APP_DESC_OFFSET = 0x20
APP_DESC_MAGIC = 0xABCD5432
SLOT_STATES = ("NONE", "NEW", "PENDING_VERIFY", "VALID", "INVALID", "ABORTED", "UNDEFINED")


def image_info(path):
    """Reads an application image and returns its bytes and the facts an update checks."""
    data = Path(path).read_bytes()
    if len(data) < APP_DESC_OFFSET + 176 or data[0] != 0xE9 \
            or struct.unpack_from("<I", data, APP_DESC_OFFSET)[0] != APP_DESC_MAGIC:
        raise ValueError(f"{path} is not an ESP application image")
    desc = data[APP_DESC_OFFSET:APP_DESC_OFFSET + 256]
    return data, {"path": str(path), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                  "elf_sha256": desc[144:176].hex(),
                  "project_name": desc[48:80].split(bytes(1))[0].decode("ascii", errors="replace")}


class Deck:
    """One serial session on the deck for an update: link requests, console lines, and reopening after a reset."""

    def __init__(self, device, log_path=None):
        self.device = device
        self.ser = None
        self.decoder = None
        self.seq = 0
        self.partial = bytearray()
        self.lines = []
        self.start = time.monotonic()
        self.transcript = open(log_path, "w", encoding="utf-8", newline="\n") if log_path else None

    def stamp(self):
        return f"[+{time.monotonic() - self.start:7.3f}s]"

    def note(self, text):
        if self.transcript:
            self.transcript.write(f"{self.stamp()} >> {text}\n")
            self.transcript.flush()

    def open(self):
        from touchdeck_serial import open_port
        self.ser = open_port(self.device)
        self.ser.timeout = 0.01
        self.ser.write_timeout = 2
        self.ser.reset_input_buffer()
        self.decoder = StreamDecoder()
        self.partial = bytearray()

    def close(self):
        if self.ser is not None:
            try:
                self.ser.close()
            except OSError:
                pass
        self.ser = None

    def pump(self):
        """Reads what is waiting. Returns the frames, and keeps the console lines in self.lines."""
        data = self.ser.read(self.ser.in_waiting or 1)
        text, frames = self.decoder.feed(data)
        if text:
            self.partial.extend(text)
            *ready, rest = bytes(self.partial).split(b"\n")
            self.partial = bytearray(rest)
            for raw in ready:
                line = raw.decode("utf-8", errors="replace").rstrip("\r")
                self.lines.append(line)
                if self.transcript:
                    self.transcript.write(f"{self.stamp()} {line}\n")
                    self.transcript.flush()
        return frames

    def request(self, message, timeout=2.0):
        self.seq = self.seq % 255 + 1
        seq = self.seq
        payload = json.dumps(message, separators=(",", ":")).encode()
        wire = encode_frame(1, seq, 0, payload)
        if self.ser.write(wire) != len(wire):
            raise ValueError("short write")
        self.ser.flush()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for frame in self.pump():
                if frame["type"] == 1 and frame["flags"] == 1 and frame["seq"] == seq:
                    return validate_reply(frame, seq, message)
        raise TimeoutError(f"no reply to {message['m']} within {timeout:g}s")

    def console(self, line, cmd, timeout=3.0):
        """Runs one console line and returns the `@td` reply whose cmd is `cmd`."""
        from touchdeck_serial import REPLY_RE
        self.lines.clear()
        self.partial = bytearray()
        self.ser.write(line.encode("utf-8") + b"\n")
        self.ser.flush()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump()
            while self.lines:
                match = REPLY_RE.match(self.lines.pop(0))
                if match:
                    reply = json.loads(match.group(1))
                    if reply.get("cmd") == cmd:
                        return reply
        raise TimeoutError(f"no reply to {line!r} within {timeout:g}s")

    def send_image(self, xfer, data, timeout):
        """Sends the chunks through the window of four with cumulative acks, as the deck's receiver expects."""
        chunks = [data[i:i + CHUNK_DATA_MAX] for i in range(0, len(data), CHUNK_DATA_MAX)]
        total = len(chunks)
        next_idx = 0
        sent_at, count = {}, {}
        retransmits = 0
        seq = 0
        deadline = time.monotonic() + timeout
        started = time.monotonic()
        while next_idx < total:
            now = time.monotonic()
            if now > deadline:
                raise TimeoutError(f"transfer stopped at chunk {next_idx} of {total}")
            for idx in range(next_idx, min(next_idx + UPDATE_WINDOW, total)):
                due = idx not in sent_at
                if not due and now - sent_at[idx] >= UPDATE_RETRY_S:
                    if count[idx] >= UPDATE_TRANSMISSIONS:
                        raise ValueError(f"chunk {idx} had no ack after {UPDATE_TRANSMISSIONS} transmissions")
                    retransmits += 1
                    due = True
                if due:
                    seq = seq % 255 + 1
                    payload = bytes([xfer, idx & 0xFF, idx >> 8]) + chunks[idx]
                    wire = encode_frame(2, seq, 0, payload)
                    if self.ser.write(wire) != len(wire):
                        raise ValueError("short write")
                    sent_at[idx] = now
                    count[idx] = count.get(idx, 0) + 1
            for frame in self.pump():
                if frame["type"] == 3 and frame["flags"] == 1:
                    ack = bytes.fromhex(frame["payload_hex"])
                    if ack[0] == xfer:
                        acked = ack[1] | ack[2] << 8
                        if acked > next_idx:
                            next_idx = min(acked, total)
        seconds = time.monotonic() - started
        return {"chunks": total, "retransmits": retransmits, "seconds": round(seconds, 3),
                "bytes_per_s": round(len(data) / seconds) if seconds else None}

    def state(self):
        """`update state` over the console, with the slot states keyed by slot name."""
        return self.console("update state", "update", 3.0)

    def wait_hello(self, deadline):
        """Answers the first hello the deck gives. A reset takes the port away, so this opens it again as often as
        it must. A deck that is still booting answers nothing for a moment, and that is not a reason to close."""
        from touchdeck_serial import wait_for_port
        import serial
        while time.monotonic() < deadline:
            try:
                if self.ser is None:
                    if not wait_for_port(self.device, timeout=1.0, settle=0.5):
                        continue
                    self.open()
                return self.request({"m": "hello"}, 0.6)
            except (serial.SerialException, OSError):
                self.close()
                time.sleep(0.25)
            except (TimeoutError, ValueError):
                time.sleep(0.25)
        raise TimeoutError("the deck did not answer hello again")


def update_command(args):
    """Updates the deck with an application image over the link. Exit 0 only when every check below holds."""
    import serial
    from touchdeck_serial import resolve_port
    report = {"ok": False, "expect": args.expect, "wait": args.wait}
    deck = None
    try:
        data, report["image"] = image_info(args.image)
        device = resolve_port(args.port)
        report["port"] = device
        deck = Deck(device, args.log)
        deck.open()
        before = deck.request({"m": "hello"}, args.timeout)
        report["before"] = {"hello": before}
        deadline = time.monotonic() + 30
        state = deck.state()
        while not state.get("confirmed") and time.monotonic() < deadline:
            time.sleep(1.0)
            state = deck.state()
        report["before"]["state"] = state
        begin = deck.request({"m": "update.begin", "bytes": len(data), "sha256": report["image"]["sha256"]},
                             args.timeout)
        report["begin"] = begin
        if not begin["ok"]:
            report["error"] = f"update.begin refused: {begin.get('reason')}: {begin.get('error')}"
            return finish_update(report, args, 1)
        report["transfer"] = deck.send_image(begin["xfer"], data, args.transfer_timeout)
        end = deck.request({"m": "update.end", "xfer": begin["xfer"]}, 20.0)
        report["end"] = end
        if not end["ok"]:
            report["error"] = f"update.end refused: {end.get('reason')}: {end.get('error')}"
            return finish_update(report, args, 1)
        if not args.wait:
            report["ok"] = True
            return finish_update(report, args, 0)
        # The deck restarts 250 ms after the reply. Let the old image go before looking for the new one.
        ended = time.monotonic()
        time.sleep(1.5)
        reboot_deadline = ended + args.reconnect_seconds
        settle_deadline = None
        samples = []
        slot = begin["slot"]
        while True:
            hello = deck.wait_hello(reboot_deadline)
            if settle_deadline is None:
                settle_deadline = time.monotonic() + args.valid_seconds
                report["seconds_to_hello"] = round(time.monotonic() - ended, 3)
            try:
                state = deck.state()
            except (TimeoutError, ValueError):
                state = None
            except (serial.SerialException, OSError):
                deck.close()
                state = None
            samples.append({"t": round(time.monotonic() - ended, 3), "elf_sha256": hello["elf_sha256"],
                            "running": hello["running"], "slot_state": state.get(slot) if state else None})
            report["after"] = {"hello": hello, "state": state, "samples": samples[-30:]}
            if state is not None and update_settled(args.expect, before, report["image"], slot, hello, state):
                report["seconds_to_settled"] = round(time.monotonic() - ended, 3)
                report["ok"] = True
                return finish_update(report, args, 0)
            if time.monotonic() > (reboot_deadline if args.expect == "rollback" else settle_deadline):
                report["error"] = update_failure(args.expect, before, report["image"], slot, hello, state)
                return finish_update(report, args, 1)
            time.sleep(0.5)
    except TimeoutError as error:
        report["error"] = str(error)
        return finish_update(report, args, 3)
    except (SystemExit, serial.SerialException, OSError, ValueError, TypeError, KeyError) as error:
        report["error"] = str(error)
        return finish_update(report, args, 2)
    finally:
        if deck is not None:
            deck.close()
            if deck.transcript:
                deck.transcript.close()


def update_settled(expect, before, image, slot, hello, state):
    """The state the update should end in: the new image confirmed, or the old one back with the new slot ABORTED."""
    if expect == "rollback":
        return (hello["elf_sha256"] == before["elf_sha256"] and hello["running"] == before["running"]
                and state.get(slot) == "ABORTED")
    return (hello["elf_sha256"] == image["elf_sha256"] and hello["running"] == slot
            and state.get(slot) == "VALID" and state.get("confirmed") is True)


def update_failure(expect, before, image, slot, hello, state):
    want = before if expect == "rollback" else image
    return (f"expected {expect}: wanted elf_sha256 {want['elf_sha256'][:12]}, got {hello['elf_sha256'][:12]} on "
            f"{hello['running']}; slot {slot} is {state.get(slot) if state else '?'}, "
            f"confirmed {state.get('confirmed') if state else '?'}")


def finish_update(report, args, code):
    output = json.dumps(report, indent=2)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(output + "\n", encoding="utf-8", newline="\n")
    print(output)
    return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    root = Path(os.environ.get("TOUCHDECK_ROOT", Path(__file__).resolve().parents[4]))
    test = sub.add_parser("selftest", help="check both codec directions against golden frames without a board")
    test.add_argument("--fixtures", type=Path, default=root / "schema/protocol/golden-frames.json")
    upd = sub.add_parser("update", help="update the deck's firmware over the link, see features/update.md")
    upd.add_argument("image", type=Path, help="an application image, such as firmware/build/touchdeck.bin")
    upd.add_argument("--port")
    upd.add_argument("--wait", action="store_true",
                     help="wait for the deck to restart, then check hello and update state")
    upd.add_argument("--expect", choices=("update", "rollback"), default="update",
                     help="update: the new image runs in the other slot and is VALID. "
                          "rollback: the old image is back and the new slot is ABORTED")
    upd.add_argument("--timeout", type=float, default=2.0, help="seconds to wait for each reply")
    upd.add_argument("--transfer-timeout", type=float, default=120.0, help="seconds the image may take to send")
    upd.add_argument("--reconnect-seconds", type=float, default=60.0,
                     help="seconds the deck has to answer hello again after update.end")
    upd.add_argument("--valid-seconds", type=float, default=20.0,
                     help="seconds from the first hello of the new image to update state reading VALID")
    upd.add_argument("--log", type=Path, help="save every console line of the session, with time offsets")
    upd.add_argument("--out", type=Path, help="also write the JSON report here")
    for command in ("hello", "ping", "state", "raw", "scenario"):
        item = sub.add_parser(command)
        item.add_argument("--port")
        item.set_defaults(timeout=0.5, log=None, time=False)
        if command != "scenario":
            item.add_argument("--timeout", type=float, default=0.5)
            item.add_argument("--log", type=Path, help="save console text received in this serial session")
            item.add_argument("--time", action="store_true", help="print each round trip in ms to stderr")
        if command == "ping":
            item.add_argument("--count", type=int, default=1)
        if command == "raw":
            item.add_argument("--hex", required=True)
        if command == "scenario":
            item.add_argument("file", type=Path)
            item.add_argument("--out", type=Path)
    for command, field in (("hold", None), ("widget-set", "widget.set"), ("dashboard-set", "dashboard.set")):
        item = sub.add_parser(command, help="say hello and keep the link online" if field is None
                              else f"say hello, send {field} with the JSON fields given, and keep the link online")
        if field is not None:
            item.add_argument("fields", help="JSON object of the message's fields, without m")
        item.add_argument("--port")
        item.add_argument("--timeout", type=float, default=0.5)
        item.add_argument("--seconds", type=float, default=10 if field is None else 0,
                          help="seconds to keep the link online after hello")
        item.add_argument("--shot", type=Path, help="save one screenshot PNG taken while the link is online")
        item.add_argument("--shot-at", type=float, default=1.5, help="seconds after hello to take the screenshot")
        item.add_argument("--scale", type=int, default=1)
        item.set_defaults(message=field, log=None, time=False)
    args = parser.parse_args()
    if args.command == "selftest":
        try:
            print(json.dumps(selftest(args.fixtures)))
            return 0
        except (AssertionError, ValueError, KeyError, OSError) as error:
            print(json.dumps({"ok": False, "error": str(error)}))
            return 1
    if args.command == "update":
        if args.expect == "rollback" and not args.wait:
            parser.error("--expect rollback needs --wait")
        return update_command(args)
    if args.timeout <= 0 or getattr(args, "count", 1) < 1:
        parser.error("timeout and count must be positive")
    import serial
    from touchdeck_serial import open_port, resolve_port
    transcript = None
    try:
        scenario = None
        session = None
        if getattr(args, "message", None) is not None or args.command == "hold":
            session = []
            if args.command != "hold":
                fields = json.loads(args.fields, parse_constant=reject_constant)
                if not isinstance(fields, dict):
                    raise ValueError("fields must be a JSON object")
                session.append(dict(fields, m=args.message))
        if args.command == "scenario":
            scenario = json.loads(args.file.read_text(encoding="utf-8-sig"), parse_constant=reject_constant)
            scenario_actions(scenario)
        device = resolve_port(args.port)
        if args.log:
            args.log.parent.mkdir(parents=True, exist_ok=True)
            transcript = args.log.open("w", encoding="utf-8", newline="\n")
        with open_port(device) as ser:
            ser.timeout = min(0.01, args.timeout)
            ser.write_timeout = args.timeout
            ser.reset_input_buffer()
            if session is not None:
                result = online_session(ser, args, session)
                result["port"] = device
                print(json.dumps(result, indent=2))
                return 0 if result["ok"] else 1
            if scenario is not None:
                result = run_scenario(ser, scenario)
                result["port"] = device
                output = json.dumps(result, indent=2)
                if args.out:
                    args.out.parent.mkdir(parents=True, exist_ok=True)
                    args.out.write_text(output + "\n", encoding="utf-8", newline="\n")
                print(output)
                return 0 if result["ok"] else 1
            decoder = StreamDecoder()
            for i in range(getattr(args, "count", 1)):
                seq = i % 255 + 1
                request = None
                if args.command == "raw":
                    wire = bytes.fromhex(args.hex)
                    try:
                        sent = decode_frame(wire)
                        seq = sent["seq"]
                    except ValueError:
                        seq = None
                else:
                    request = {"m": "link.state" if args.command == "state" else args.command}
                    if args.command == "ping":
                        request["n"] = i
                    wire = encode_frame(1, seq, 0, json.dumps(request, separators=(",", ":")).encode())
                reply, elapsed = exchange(ser, decoder, wire, args.timeout, seq, request, transcript)
                print(json.dumps(reply, separators=(",", ":")), flush=True)
                if args.time:
                    print(f"rtt_ms: {elapsed:.3f}", file=sys.stderr)
                if not reply["ok"]:
                    return 1
        return 0
    except TimeoutError as error:
        print(json.dumps({"ok": False, "error": str(error)}))
        return 3
    except (SystemExit, serial.SerialException, OSError, ValueError, TypeError, KeyError) as error:
        print(json.dumps({"ok": False, "error": str(error)}))
        return 2
    finally:
        if transcript:
            transcript.close()


if __name__ == "__main__":
    sys.exit(main())
