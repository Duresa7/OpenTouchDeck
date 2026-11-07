# TouchDeck app link v1

The app link shares the debug console's USB-Serial/JTAG byte stream. Text commands
and their `@td` replies remain available between binary frames. The device uses
VID `303a` and PID `1001`. Opening the port does not reset it.

## Frame layout

A frame begins and ends with `0xC0`. An encoder writes both delimiters on every
frame, including when two frames are adjacent. Within the body, `0xC0` becomes
`0xDB 0xDC` and
`0xDB` becomes `0xDB 0xDD`. No other escape is valid. `0xC0` is not a valid UTF-8
byte, so the receiver recognizes frames without a text-mode command. A literal
`0xDB` outside a frame remains text.

The decoded body has these fields in order.

| Offset | Field | Encoding |
|---|---|---|
| 0 | `ver` | u8, always 1 |
| 1 | `type` | u8, message 1, chunk 2, ack 3 |
| 2 | `seq` | u8, copied from the request into its response |
| 3 | `flags` | u8, request 0, response 1, event 2 |
| 4 | `len` | u16le, decoded payload length, 0 to 1024 |
| 6 | `payload` | Exactly `len` bytes |
| 6 + `len` | `crc` | u16le, CRC-16/CCITT-FALSE of bytes 0 through the payload |

CRC-16/CCITT-FALSE uses polynomial `0x1021`, initial value `0xFFFF`, no
reflection, and no final XOR. Its check value for ASCII `123456789` is `0x29B1`.

A maximum decoded body is 1032 bytes. A receiver unescapes incrementally into a
decoded buffer of at least 1032 bytes, and the deck's holds 1100. An encoder
reserves 2066 wire bytes, twice the maximum body plus both delimiters, but no
valid frame fills them, because `ver`, `type`, `flags`, and a `len` of 1024
never need escapes. The largest frame is 2061 bytes, a 1024-byte payload whose
`seq`, payload, and both CRC bytes all need escapes. The
`maximum-escaped-response` fixture is one. The `maximum-escaped-chunk` fixture
is 2058 bytes, because its `seq` and CRC need none.

## Receiving

The deck applies these rules to the bytes it reads. A host decoder applies them
too, plus the host rules below.

1. After a delimiter, the next byte decides. `0x01`, the only valid `ver`,
   starts a frame body. Another `0xC0` changes nothing, so a run of delimiters
   is harmless. Any other byte means no frame opened, and that byte goes to the
   text path.
2. A frame closes only when its body fully decodes. A delimiter ends the body in
   progress and hands it to the decoder. A body that fails to decode, whatever
   its length, increments `crc_errors` and gets no reply. So does a body with a
   bad escape or more than 1100 decoded bytes.
3. The delimiter that ends a body, decoded or not, also opens the next frame. A
   closed frame followed at once by `0x01` therefore opens the next frame, as if
   a delimiter came between, and a lost closing delimiter costs one frame at
   most. Text right after a closed frame, or after a body that failed to
   decode, reaches the console at once, because its first byte is not `0x01`.
4. The frame deadline runs from the opening delimiter. A frame still open
   500 ms after its opener goes back to text, however much text arrives
   meanwhile, and increments `crc_errors` if it held a byte. The deck's read
   timeout is the time left before that deadline. A host writes each frame
   without pausing.
5. The console refuses a text line that holds a control byte other than tab,
   CR, LF, backspace, or DEL. It answers `ok:false` with
   `error:"control character"` and runs nothing. The console's existing NUL
   mapping stays, so a NUL from the host becomes `?` before this check.

`crc_errors` counts every body these rules drop: a bad CRC, an invalid header,
a wrong length, a bad escape, an overflow, and a deadline. A body whose first
byte is not `0x01` is text and counts nothing.

A host decoder also returns to text as soon as 6 decoded bytes are in and the
header is invalid, meaning a `type` outside 1 to 3, `flags` outside 0 to 2, an
event with a nonzero `seq`, a `len` over 1024, a chunk under 3 bytes, or an ack
that is not 3 bytes. It does the same when the decoded size passes `len` plus 8,
and on a bad escape. It gives the raw bytes of every dropped body back to its
text stream, because nothing on the host runs them. An idle flush returns a held
frame's bytes to text 500 ms after its opener, so no console reply waits behind
a dead frame. Each counts one error, as rule 2 does on the deck.

A host that stalls past the deadline inside a frame loses that frame, and the
rest of its bytes reach the console as text. Rule 5 refuses a line that holds a
header or CRC byte below `0x20`. A chunk whose data holds a whole printable line
after the stall point, such as `\nreboot\n`, would still run that line.

Only flags 0, 1, and 2 are valid. An event has `seq` 0. Requests may use any u8
sequence, though clients normally cycle through 1 to 255. A client keeps a
sequence unique among outstanding requests and matches replies by sequence.

## Payloads

| Type | Payload |
|---|---|
| Message, 1 | One UTF-8 JSON object with a string `m` field |
| Chunk, 2 | `xfer` u8, `idx` u16le, then 0 to 1021 data bytes |
| Ack, 3 | `xfer` u8, then `next` u16le, exactly 3 bytes |

A maximum chunk has 1021 data bytes and a 1024-byte payload. The codec validates
chunk and ack lengths. Chunk transfers, acks, and events are described under
Transfers, settings, and events below.

Transfers use a window of four unacknowledged chunks. `idx` starts at zero, and
`next` acknowledges every index below it and names the next contiguous index
the receiver needs. An ack identifies the transfer by `xfer`. Its frame sequence
echoes the request that caused it. A sender retransmits unacknowledged chunks
after 200 ms, with at most five transmissions per chunk, including the first.
A receiver must accept a duplicate chunk without applying its data twice.

## Messages

A response is a type 1 frame with flags 1 and the request's sequence. It carries
the same `m`, a Boolean `ok`, and the fields below. All times are milliseconds
since boot. Counters reset at boot.

| Request | Successful response fields |
|---|---|
| `{"m":"hello"}` | `product:"touchdeck"`, `proto:1`, `elf_sha256`, `version`, `running`, `sinks:["log","link"]`, `recovery:false`, `layout:"builtin"` or `"storage"`, `carrier` |
| `{"m":"ping","n":N}` | `t`, and `n` echoed when it is a number, string, Boolean, or null |
| `{"m":"link.state"}` | `online`, `frames_rx`, `frames_tx`, `crc_errors`, `dropped`, `last_rx_ms`, `carrier` |

`elf_sha256` is the running app descriptor's 64-character lowercase hex hash.
`version` is its build version. `running` is the OTA partition label, `ota_0` or
`ota_1`. `ping` omits `n` when the request has none, or when
it is an array or an object.

A reply echoes at most the first 64 characters of `m`. An unknown `m` returns
`ok:false` and `error:"unknown message"`. A payload that cannot be parsed as one
object with a string `m` produces no response and increments `dropped`. JSON
nested deeper than 16 levels does not parse, so a frame of 1024 `[` bytes is
dropped this way. An unsupported incoming frame type or flags, a full frame
queue, or a response that does not fit one 1024-byte payload also increments
`dropped`. A `ping` whose string `n` nears 1000 bytes gets no reply for that
reason.

`frames_rx` counts valid frames before dispatch, including frames later dropped.
`frames_tx` counts complete response writes. A `link.state` reply therefore
includes its request in `frames_rx` and excludes itself from `frames_tx`.
`last_rx_ms` records the most recent valid frame's arrival time. `online` becomes
true on `hello` and expires after 5000 ms without a valid frame. After expiry,
another `hello` establishes the connection again.

`carrier` names how frames travel: `usb`, or `wifi` for net-v2 over TCP in
the radio build. In `hello` it is the carrier the request arrived on. In
`link.state` it is the active carrier, the one events leave on. A response always
leaves on the carrier its request arrived on. Over a network carrier only the
messages [net-v2](net-v2.md) lists are served; any other gets `ok:false`,
`reason:"usb-only"` and `error:"Only over USB"`.

The text command `link state` returns the same state fields in the console's
usual `{"cmd":"link","ok":true,...}` envelope.

## Transfers, settings, and events (M2-05)

Every message below follows the response rules above. A refusal is `ok:false` with an `error` string.

| Request | Successful response fields |
|---|---|
| `{"m":"layout.put","orientation":"landscape"\|"portrait","bytes":N,"crc32":C}` | `xfer`, the id of the transfer it opened. `bytes` is 1 to 32768. |
| `{"m":"layout.get","orientation":O}` | `xfer`, `bytes`, `crc32` of the stored file. `ok:false` with `no stored layout for <O>` when the slot is empty. The deck then sends the chunks. |
| `{"m":"icon.put","id":"<16 lowercase hex>","bytes":12300,"crc32":C}` | `xfer`. The file is a `TDI1` icon, and its header is checked at `xfer.end`. |
| `{"m":"icon.list","cursor":N}` | `ids`, at most 32 ids, and `next`, the cursor for the following page or `null` at the end. A missing `cursor` is 0. |
| `{"m":"icon.rm","id":"<16 hex>"}` | no extra fields. `no such icon` when absent. |
| `{"m":"settings.get"}` | `idle_timeout_s`, `orientation` |
| `{"m":"settings.put","idle_timeout_s":S,"orientation":O}` | the stored `idle_timeout_s` and `orientation`, and `reboot_required`, true when the orientation changed. A field left out keeps its stored value. The idle timeout applies at once. |
| `{"m":"store.stat"}` | `total_bytes`, `used_bytes`, `layouts` and `icons`, the file counts of `/storage/layouts` and `/storage/icons` |
| `{"m":"xfer.end","xfer":X}` | After a layout put: the `layout state` fields `source`, `name`, `orientation`, `pages`, `tiles`, `icons_loaded`, `icons_missing`, `bytes`, `load_ms`, `error`. After an icon put: `id`. After a get: nothing extra. |

One transfer is open at a time, and a new `put` or `get` replaces the open one. The deck holds a put in RAM
and writes it only at `xfer.end`, so a transfer that is cut leaves nothing in storage. A put that sees no chunk for 2
seconds is dropped. At `xfer.end` the deck checks the received size and CRC-32 (`received N of M bytes`, `crc32
mismatch`), then parses and validates the file as a layout or checks an icon header. Only then does it swap the file in
with `td_store_write_atomic`. A bad file never replaces a good one. A layout for the orientation the deck booted in is
then loaded and shown without a reboot. A layout for the other orientation is stored and changes nothing on screen.
`hello` now reports `sinks:["log","link"]` and the layout source in `layout`.

**Chunks.** A put is a run of chunk frames from the app. Each is a request, type 2 with flags 0 and a fresh `seq`. The
deck answers each with an ack, type 3 with flags 1 and the chunk's `seq`, naming the next index it wants. A duplicate, a
gap, and a chunk past the window of four all get the same answer, with no data applied. A get runs the other way: the
deck sends chunk requests with its own `seq`, and the app answers each with an ack that echoes it. The sender
retransmits a chunk after 200 ms, five transmissions in all, then aborts.

**Events.** An event is a message with flags 2 and `seq` 0. The deck sends events only while the link is online, and
throws them away otherwise.

| Event | Fields |
|---|---|
| `press` | `page`, `tile`, `kind` (`launch`, `open`, `command`, or `widget`), `seq`, `t`. `seq` counts presses since boot. |
| `key.report` | `seq`, `t`, `type`, and `mods` and `keys` for `kbd`, or `usage` for `con`. `seq` is the key sink's count since boot, equal to the console key log's, so the app drops a duplicate and sees a gap. |
| `key.resync` | `seq`, `t`, `mods`, `keys`, `usage`: the last state the sink saw. |

The link sink queues reports without blocking, 64 deep, and never loses a release. When the queue is full it throws the
queued reports away and sends one `key.resync` before anything newer. A consumer releases every key it holds on a `seq`
gap, then applies the next report, and sets its state exactly on a `key.resync`.

## Firmware update (M2-09)

| Request | Successful response fields |
|---|---|
| `{"m":"update.begin","bytes":N,"sha256":"<64 hex>"}` | `xfer`, the id of the transfer it opened, and `slot`, the OTA slot the image is written to. `bytes` is 1 to 4194304, the size of a slot. |
| `{"m":"update.end","xfer":X}` | `slot`, the new boot slot. The deck restarts 250 ms after the reply. |

An update is a transfer like a layout put: the app sends the image as chunk frames and the deck acks each one. The deck
writes every chunk into the slot the running image does not occupy as it arrives, and hashes it, so it holds no copy. At
`update.end` it checks that every byte arrived and that the SHA-256 of the written bytes equals the one in `update.begin`,
validates the image, checks that it is a `touchdeck` image, sets the new slot as the boot slot, replies, and restarts.

A refusal is `ok:false` with an `error` sentence and a `reason` word, and it leaves the boot slot as it was.

| `reason` | Meaning |
|---|---|
| `args` | `bytes` or `sha256` is missing or not the right type |
| `size` | `bytes` is 0 or larger than the slot |
| `sha256-format` | `sha256` is not 64 hex digits |
| `busy` | another update is receiving, verifying, or restarting |
| `unconfirmed` | the running image has not confirmed itself yet |
| `state` | `update.end` named a transfer that is not open |
| `incomplete` | `update.end` came before every byte arrived |
| `sha256` | the hash of the written bytes is not the expected one |
| `flash` | the flash or the image check refused the write |
| `project` | the image is for another project |

`xfer.end` on an update's transfer is refused. A transfer that sees no chunk for 2 seconds is dropped and the update
returns to `IDLE`. The text command `update state` reports the machine as `state`: `IDLE`, `RECEIVING`, `VERIFYING`,
`REBOOTING`, or `FAILED` with a `reason`, and the two slots' `esp_ota_img_states_t` names.

The new image boots as `PENDING_VERIFY` and confirms itself after the first frame, a layout load, and 10 seconds of
uptime. A reset before that rolls back to the previous slot, and the bootloader marks the new one `ABORTED`.
`update.begin` is refused while the running image waits to confirm.

## Widgets and dashboards (M2-06)

| Request | Successful response fields |
|---|---|
| `{"m":"widget.set","tile":T,"value":V,"caption":C,"bar":B,"stale":S}` | `tile`, and the stored `value`, `caption`, `bar`, and `stale` |
| `{"m":"dashboard.set","page":P,"rows":[{"label":L,"value":V,"bar":B}]}` | `page`, `rows`, the number of rows stored |

`tile` names a `widget` tile of the layout on screen, and `page` a `dashboard` page. Any other id is refused with
`unknown tile` or `unknown page`. `value`, `caption`, `label`, and `bar` are optional. A missing string is empty, and
a missing or null `bar` is -1, meaning no bar. A `bar` is a fraction from 0 to 1. The deck stores a negative bar as -1
and a bar above 1 as 1. It keeps the first 24 bytes of `value`, 16 of `caption`, and 16 of each row's `label` and
`value`, cut on a UTF-8 boundary. `stale` is false when missing, and the deck draws a stale value at 60 percent
opacity. More than 8 rows are refused whole with `too many rows`. A field of the wrong type is refused. A set replaces
the whole state of its tile or page.

The link task writes a set into the deck's state table and marks the tile dirty. A 100 ms redraw timer draws each
dirty tile once, so a burst of sets costs one redraw. A set is stored whether or not the link is online. The state
table starts empty for every layout the deck loads.

While the link is offline, the deck draws every `launch`, `open`, `command`, and `widget` tile at 40 percent opacity.
A tap on a dimmed tile logs `ui: offline <tile>` and sends no `press` event. The deck checks the link on its redraw
timer, so tiles brighten within 100 ms of `hello` and dim within 100 ms of the 5000 ms expiry.

## Concurrency and discovery

The RX task splits text and frame bodies before text NUL mapping or overflow
handling. The console consumes text at priority 2. The `td_link` task consumes
the frame stream at priority 3 with a 6144-byte stack. Each channel preserves
its own input order. There is no cross-channel reply-order guarantee. For
example, a ping may answer before an earlier long console line is rejected.
Asynchronous console commands such as `perf fps` retain their existing delayed
reply behavior.

The console never waits for a link handler. The RX task hands each frame to the
link's 8192-byte stream buffer without blocking and counts a frame that does
not fit in `dropped`. The two tasks share only the stdout lock, and each holds
it for one complete write, so a slow link handler cannot stall a text command.

Frames, console replies, prompts, and echo use `td_console_write` or its line
variant under the stdout lock. ESP-IDF log formatting uses that same lock.
A complete frame cannot interleave with a text write or log line. A console
stream can contain frames between its writes, so clients demultiplex bytes
before splitting the text into lines.

Discovery probes every serial port with VID `303a` and PID `1001`. A candidate
is a deck only if it answers `hello` within 500 ms with product `touchdeck` and
protocol 1. The client opens without toggling DTR or RTS. Windows opens COM3
exclusively, so mixed traffic uses one port handle and one receive decoder.

GPIO19, GPIO20, the USB-Serial/JTAG controller, its clock, and its power remain
untouched. This milestone adds no light sleep, deep sleep, eFuse write, or
configuration that disables USB-Serial/JTAG. Unattended recovery through the
ROM's reset into download mode depends on those invariants.

## Golden frames

[golden-frames.json](golden-frames.json) supplies hex wire bytes and decoded
fields to the firmware tests, Python selftest, and the later Rust codec. A
fixture's `valid` field describes framing, not message dispatch. For example,
an empty message payload is a valid frame but not a dispatchable JSON object.

The fixtures include every escape, a maximum chunk with 1021 data bytes, a
maximum payload made entirely of `0xC0` and `0xDB`, the largest escaped frame,
a chunk with no data, and
invalid CRC, version, length, and escape cases. Tests compare exact bytes in
both codec directions and preserve frame state across fragmented reads.

Each entry in `streams` is a run of wire bytes that a stream decoder reads at
any read size. `frames` names, in order, the fixtures it must yield, and
`errors` counts the bodies it must drop. `truncated-then-valid` is a frame cut
off before its closing delimiter and followed at once by a valid frame, so the
valid frame's opening delimiter ends the cut one. A stream holds no text, so the
deck passes nothing to its console. A host decoder returns the dropped bytes as
text, as the receive rules say.
