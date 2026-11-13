# App link

The deck accepts framed `hello`, `ping`, and `link.state` messages beside its text console on USB-Serial/JTAG. One serial connection can carry both channels. The local Python client validates CRCs and responses without resetting the board.

## Sub-features

- `link-identity` reports the product, protocol, app hash, version, running partition, sinks, recovery flag, and layout.
- `link-ping` echoes `n` with the device's uptime in milliseconds.
- `link-state` reports online state, traffic counters, errors, drops, and the last valid frame's arrival time. The console command `link state` exposes the same fields.
- `link-codec` checks golden frames in both directions without a board, including a maximum chunk with 1021 data bytes and a maximum payload containing only escaped bytes.
- `link-scenario` sends scheduled text, raw bytes, and frames through one open port. It records console replies, log lines, sequence-matched link responses, events, and key records in one JSON result.

## How to get to it (user POV)

- Run `$TD link hello`, `$TD link ping --count 100`, or `$TD link state`.
- Run `$TD serial cmd "link state"` from the text side.
- Run `$TD link raw --hex <frame>` to send exact wire bytes. The hex string includes both delimiters.
- Run `$TD link selftest` for PC codec and scenario-runner checks.
- Run `$TD link scenario <file> --out <absolute-result-path>` for mixed traffic. The JSON file defines the sends and expectations below.

## Driving it with touchdeck.ps1

Preconditions:

- Use PowerShell. Set `TOUCHDECK_ROOT` to the tested worktree and pass absolute output paths.
- Build once at a time. Hold the board lock for every live operation. Follow the milestone 2 Boot recipe and save Doctor exit 0 with `build_matches:true` before a live lane.
- Set `$E` to the lane's absolute evidence directory. Save each recipe's scenario JSON there before running it.
- Keep GPIO19, GPIO20, the USB-Serial/JTAG controller, its clock, and its power unchanged. Add no light sleep, deep sleep, eFuse write, or configuration that disables USB-Serial/JTAG. Unattended ROM recovery depends on all of those invariants.

- PC selftest. Run `$TD link selftest`. Require exit 0 and `ok:true`. It checks the golden file, fragmented reads, both cross-channel reply orders, sequence wrap, events, and missing or mismatched responses with a simulated serial endpoint. It never opens a port and does not prove firmware behavior on hardware.
- Identity and lane 2. Run `$TD link hello > $E/hello.json`. Require product `touchdeck`, protocol 1, running partition `ota_0`, sinks `["log"]`, and the same `elf_sha256` as Doctor's `local_build.elf_sha256`. Run `$TD serial cmd "version"` afterwards and require a successful reply on the same port.
- Pings and lane 3. Run `$TD link ping --count 200 > $E/ping.jsonl`. Require 200 successful replies with `n` from 0 to 199, then run `$TD serial cmd "link state"` and require `crc_errors` 0. The You-see check uses `--count 100` and expects 0 to 99.
- Online timeout. Run `hello`, then read the text `link state` after 3 seconds and after 7 seconds from hello. Require true, then false. A framed `link.state` refreshes the receive time and cannot measure silence.
- Bad CRC. Send the `bad-crc` fixture through `link raw`. Require exit 3, which means no reply within 500 ms. Follow with `hello` and the text `link state`. Require a successful hello and one additional `crc_errors`.
- Deep JSON. Save the scenario below and run `$TD link scenario $E/deep-json.json --out $E/deep-json.result.json`. Require exit 0. Between the two `link` replies, `crc_errors` stays put and `dropped` rises by one. The runner's heartbeat check proves the deck kept running.

```powershell
$scenario = @{
    duration_ms = 7000; timeout_ms = 500; heartbeat_gap_ms = @(1800, 2800); heartbeat_min = 3
    steps = @(
        @{ id = 'before'; text = 'link state'; expect = @{ cmd = 'link'; ok = $true } }
        @{ id = 'deep'; after = 'before'; at_ms = 200; frame = @{ type = 1; seq = 50; payload_hex = ('5b' * 1024) }; expect_reply = $false }
        @{ id = 'after'; after = 'deep'; at_ms = 1000; text = 'link state'; expect = @{ cmd = 'link'; ok = $true } }
        @{ after = 'after'; at_ms = 6500; text = 'version' }
    )
}
$scenario | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 $E/deep-json.json
```

- Cut frame, then text. Save the scenario below and run `$TD link scenario $E/cut-frame.json --out $E/cut-frame.result.json`. Require exit 0. Compute each `uptime` send's age from the `cut[0]` send in `sent`. Require one `uptime` console reply for each line aged 500 ms or more, none for the younger lines, one more `crc_errors` in the second `link` reply, and a successful `version`. The cut frame is the first 11 bytes of `hello` with `seq` 4, sent 50 ms off the line grid so no line lands on the deadline.

```powershell
$scenario = @{
    duration_ms = 3500; timeout_ms = 500
    steps = @(
        @{ id = 'before'; text = 'link state'; expect = @{ cmd = 'link'; ok = $true } }
        @{ id = 'cut'; at_ms = 250; hex = 'c0010104000d007b226d22'; expect_reply = $false }
        @{ id = 'lines'; at_ms = 300; every_ms = 100; count = 20; text = 'uptime'; expect_reply = $false }
        @{ id = 'after'; at_ms = 2800; text = 'link state'; expect = @{ cmd = 'link'; ok = $true } }
        @{ at_ms = 2900; text = 'version' }
    )
}
$scenario | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 $E/cut-frame.json
```

- Lane 6, mixed traffic. Create the scenario below and run `$TD link scenario $E/lane6.json --out $E/mixed.json`. Require exit 0. Its console replies must reject the long line before answering `version`. The ping response must match its sequence. Either channel may answer first.

```powershell
$scenario = @{
    duration_ms = 2000
    timeout_ms = 500
    steps = @(
        @{ text = ('x' * 300); expect = @{ cmd = ('x' * 63); ok = $false; error = 'line too long' } }
        @{ message = @{ m = 'ping'; n = 6 } }
        @{ text = 'version' }
    )
}
$scenario | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 $E/lane6.json
```

- Lane 8, pings and heartbeat. Save the following as `$E/lane8.json`. Run `$TD link scenario $E/lane8.json --out $E/flood.json`. Require exit 0, 2000 matched responses with `n` 0 through 1999, and every heartbeat gap within 1800 to 2800 ms. The runner derives gaps from the device's log timestamps and requires at least three heartbeats. Use this one process for both sending and capture.

```json
{
  "duration_ms": 30000,
  "timeout_ms": 500,
  "window": 1,
  "heartbeat_gap_ms": [1800, 2800],
  "heartbeat_min": 3,
  "steps": [
    {"message": {"m": "ping", "n": 0}, "count": 2000, "every_ms": 1}
  ]
}
```

- Lane 9, tap during pings. Before opening the scenario, read `power state`, wake through the gap at `touch tap 266 128` if needed, and set `power idle-timeout 86400`. Run `keys clear all`, `ui goto 0`, and `ui state`. Require page 0 at rest. Confirm `ui tile f5` has centre 143,248 on this build. Save the following as `$E/lane9.json` and run `$TD link scenario $E/lane9.json --out $E/tap-under-link.json`. Require exit 0 and 500 matched pings. The runner also requires the two F5 record entries and `deliver_us` below 60000. Restore `power idle-timeout 600` afterwards, including after a failed scenario.

```json
{
  "duration_ms": 10000,
  "timeout_ms": 1000,
  "record_brief": ["kbd 0 [62]", "kbd 0 []"],
  "steps": [
    {"id": "record", "text": "keys record start"},
    {"id": "pings", "after": "record", "message": {"m": "ping", "n": 0}, "count": 500, "every_ms": 5},
    {"id": "tap", "after": "record", "at_ms": 500, "text": "touch tap 143 248", "max": {"deliver_us": 59999}},
    {"id": "stop", "after": ["pings", "tap"], "text": "keys record stop"},
    {"after": "stop", "text": "keys record dump", "expect": {"cmd": "keys", "ok": true, "phase": "end", "count": 2}}
  ]
}
```

Scenario files contain `steps` and a `duration_ms`, default 10000. Each step has exactly one send kind.

| Kind | Value |
|---|---|
| `text` | One console line. The runner appends LF. |
| `hex` | Exact wire bytes as hex. No newline is added. |
| `message` | A JSON object with `m`. The runner allocates a free sequence and encodes it. |
| `frame` | An object with `type`, `seq`, optional `flags`, and `payload_hex`. |

`at_ms` schedules from the port session's start. `count` repeats a step, and `every_ms` spaces those scheduled sends. A repeated ping increments its integer `n`. `id` names a step, and `after` names one or more steps whose responses must complete first. The default `window` is one outstanding link request, configurable up to 255. It does not serialize the console with the link. An explicit sequence waits until its previous request completes before reuse.

Every send expects a response unless `expect_reply:false`. Use that setting for deliberately invalid raw frames or unimplemented chunk requests. For example, `{"hex":"c001dbc0","expect_reply":false}` sends a dangling escape. Such a send alone does not prove rejection. Follow it with a successful message and check the error counter.

The default text expectation is its command word and `ok:true`. `expect` supplies exact top-level fields instead. A `phase` expectation permits an earlier successful streaming header and waits for the named phase. `max` imposes numeric upper bounds. `timeout_ms`, default 500, limits each response. `record_brief` and `heartbeat_gap_ms` add the checks used above. A reply mismatch, timeout, invalid received frame, or unsent action makes `ok:false` and exit 1. The runner records a bad received frame or an unreadable reply line in `errors` and keeps running. Invalid input or an unavailable port exits 2. A successful run exits 0 and prints one JSON object. `--out` saves the same object.

`link hello`, `ping`, `state`, and `raw` exit 0 on a well-formed successful reply, 1 on `ok:false`, 3 when no reply arrives within `--timeout`, and 2 on invalid input or a port error.

## Gotchas

- Windows opens the serial port exclusively. Close the app before the lane. Use one scenario for simultaneous pings, capture, and console commands. Separate clients cannot share the port.
- There is no cross-channel order guarantee. The console task runs at priority 2 and the link task at priority 3. Their independent receive streams preserve order within each channel. The stdout lock protects complete writes, not the order in which tasks reach the lock. Lane 6 checks the console order and matches the frame by sequence separately.
- Asynchronous console replies such as `perf fps` keep their existing behavior. A scenario that deliberately overlaps those commands must account for their delayed replies. The supplied lanes use synchronous text commands.
- The receive rules are in "Receiving" in `schema/protocol/link-v1.md`. After a delimiter only `0x01` opens a frame, so text right after any frame or bad frame runs at once. A frame cut short swallows text until the next delimiter or until 500 ms after its own opener, whichever comes first. The console refuses a line holding a control byte other than tab, so a frame tail that reaches it after that deadline usually answers `control character`.
- The PC decoder in `touchdeck_link.py` gives a dropped frame's bytes back as text, and its idle flush releases a held frame 500 ms after the opener. A reader must keep feeding it on empty reads.
- A maximum chunk carries 1021 data bytes. Its 3-byte transfer header makes the decoded payload 1024 bytes. The receive buffer holds decoded bytes. An encoder reserves 2066 wire bytes, and the largest valid frame is 2061.
- `capture --send` remains a UTF-8 text operation. `capture --send-hex` can send bytes, but lanes 6, 8, and 9 use `link scenario` for sequence matching and one-port ownership.
- Scenario sends do not reset the board. The result includes only bytes received during that session. Run Doctor before the lane and keep its hash evidence.
- A screenshot cannot prove backlight state, and an injected tap does not exercise the glass or GT911. M2-01 sends key reports only to the console log. No F5 key reaches the PC.
