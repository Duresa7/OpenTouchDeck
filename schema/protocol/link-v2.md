# Open TouchDeck USB link v2

This is the implementation contract for issue #67. Runtime support belongs to
#80 and later. The current Firmware implements [v1](link-v1.md).
Normative MUST and MUST NOT state requirements. All sizes count bytes unless
specified otherwise. JSON examples and golden fixtures use synthetic identities.

## Framing and negotiation

The frame version remains **1**. CRC-16, SLIP, message/chunk/ack types,
1,024-byte payloads, 1,021-byte chunk data, sequence matching, text multiplexing,
and the 500 ms frame deadline are unchanged. `proto` selects application
semantics, not the frame header. No new binary type is allocated.

The Companion first sends the unchanged `{"m":"hello"}`. A v2 Device answers
with all v1 fields, `proto:1`, and `protocols:[1,2]`. This discovery reply does
not need capabilities. An old Companion therefore still sees protocol 1.
A new Companion sends `{"m":"hello","proto":2}` only after that advertisement.
The successful reply has `proto:2` and all fields below. It activates v2 for
this USB session. A plain hello selects v1 again. Either selection aborts
uncommitted v2 transfers, clears live values, and resets live sequence tracking.
Repeated selection is a new session. A malformed selector returns `args`;
an unsupported integer selector returns `unsupported` without changing state.
V1 Firmware ignores the selector and returns `proto:1`; that is never v2 success.

The Companion requires `product:"touchdeck"`, the selected protocol, and all
mandatory capabilities before sending v2 requests. An absent `protocols` means
v1. Unknown versions or capabilities never authorize guessed behavior. A missing
required v2 field makes negotiation fail; the Companion can select v1 again.
Ping remains every 2,000 ms. Expiry after 5,000 ms without a valid frame resets
selection to v1, aborts v2 transfers, and marks live values stale. Reconnect
always repeats discovery. No GPIO, clock, eFuse, or USB controller change is part
of this protocol. The same selection runs on every Carrier: over Wi-Fi and BLE,
[net-v2](net-v2.md) carries these frames and defines its own framing,
authentication and `net.*` messages. Each USB or net-v2 session selects its
protocol independently. On a network Carrier, net-v2 also limits which
requests run and adds the `usb-only` reason.

## Hello fields and limits

A selected v2 hello retains `m`, `ok`, `product`, `elf_sha256`, `version`,
`running`, `sinks`, `recovery`, and `layout` with their v1 meanings. It adds
`protocols:[1,2]`, `id`, `name`, and `caps`. `version` is the Firmware build
version, at most 32 UTF-8 bytes. `id` is the chip's factory base MAC rendered as
12 lowercase hex digits in byte order. It MUST equal the USB serial number after
removing colons and hyphens and folding ASCII case. A mismatch prevents automatic
association with a saved Device. It is identity, not authentication. This is the
same factory identity net-v2 uses as `device_id`, never a randomized interface MAC.
`name` follows the naming rules below. These fields never contain credentials.
The opt-in radio build also sends `radio:true`, and the normal build sends no
`radio` field. The Companion takes a missing field as the normal build and picks
Firmware updates from the same build.

Every capability below except `anim` and `live_images` is required. Bounds are ceilings, not promises of free
memory. A Device advertises only implemented Tile kinds and formats. Integers
are nonnegative JSON integers. The complete hello MUST fit one 1,024-byte payload;
Firmware must test its maximum escaped name and longest build version.

| `caps` field | Meaning and upper bound |
|---|---|
| `screen` | `[width,height]`, current logical pixels, each 1 to 4096 |
| `grid` | `[max_rows,max_cols]`, each 1 to 8 |
| `pages`, `tiles` | Maximum Pages per Layout, 8; Tiles per Page, 64 |
| `layout_bytes`, `layout_schemas` | At most 32768; supported versions, subset of `[1,2]`, including 1 |
| `free_bytes` | Current usable Asset space, same calculation as `v2.asset.stat` |
| `tile_kinds` | Subset of `static`, `gif`, `graph`, `bar`, `now-playing`, `photo-frame`, `clock`; includes `static` |
| `asset_bytes` | Maximum single Asset, 1 to 1048576 |
| `formats` | Subset of `tdi1`, `gif`, `jpeg`; includes `tdi1` |
| `image` | `[max_width,max_height]`, each at most screen dimensions |
| `gif_frames`, `gif_pixels` | Maximum frames and total decoded canvas pixels across all GIF frames; zero if unsupported |
| `anim` | Optional animation limits: `image: [width,height]`, `min_delay_ms`, `page_tiles`, `page_pixels`; positive integers. Present only with `gif` in `formats` |
| `live_images` | Optional maximum (at most 64) Tiles declaring an image stream across the whole Layout; absent means 0. The media profile advertises 4 |
| `photo_bytes` | Maximum JPEG transfer, 0 to 131072; zero disables photos |
| `live_hz`, `live_total_hz` | Maximum accepted updates per stream and Device per second, at most 2 and 20 |
| `series`, `text_bytes`, `streams` | Maximum series length, text size, and streams per Tile, at most 32, 128, and 8 |

Payload, chunk, transfer window, and naming limits are fixed by this document.
A capability of zero disables the corresponding feature. The Companion checks
Tile kind, format, dimensions, all size limits, and Layout schema before upload.
No v2 kind is inferred from `version` alone. On v1, only existing static Tiles,
widgets, and dashboards are available. The Companion marks unsupported kinds
as needing Firmware and retains the last compatible Layout on the Device.

## Requests and errors

Every new request is a type 1 JSON object with `m`, flags 0, and a unique pending
sequence. Replies use the same `m` and sequence, flags 1, and Boolean `ok`.
All v2 names start with `v2.`. Except hello, they require selected v2 or return
`not-negotiated`. Unknown v2 names return `unsupported` on v2 Firmware.
The unsolicited `v2.input` and `v2.page` events and optional `press.part` extend touch input on selected v2 sessions. Existing key events remain unchanged.

A failure is `{"m":"v2.name.set","ok":false,"reason":"args","error":"Invalid name"}`.
`reason` is stable; `error` is human-readable, at most 96 UTF-8 bytes.
Requests reject duplicate keys, unknown fields, malformed Unicode, depth over
16, and wrong types. Undecodable JSON or a missing string `m` follows v1's silent
drop rule. Valid JSON with identifiable `m` returns `args` for invalid fields.
Strings forbid U+0000 through U+001F and U+007F. Numbers in live values are the
only fields that permit fractions or exponents. No refusal changes committed
state. Fatal transfer failures discard only the temporary transfer.

All new requests can return `not-negotiated`, `args`, `unsupported`, or `memory`.
The following table lists additional reasons, exhaustively.

| Request | Additional reasons |
|---|---|
| `v2.name.get`, `v2.name.set` | `storage` |
| `v2.asset.stat`, `v2.asset.list` | `storage` |
| `v2.asset.put` | `busy`, `size`, `full`, `storage`, `format` |
| `v2.asset.end` | `state`, `incomplete`, `hash`, `format`, `size`, `full`, `storage` |
| `v2.asset.delete` | `busy`, `in-use`, `storage` |
| `v2.live.set` | `layout`, `page`, `tile`, `stream`, `type`, `size`, `asset`, `sequence`, `rate` |
| `v2.photo.put` | `busy`, `size`, `idle`, `rate` |
| `v2.photo.end` | `state`, `incomplete`, `hash`, `format`, `size`, `idle` |
| `v2.idle.get`, `v2.idle.set` | `storage` |
| `v2.clock.set` | none |

`busy` means another bulk transfer owns the slot. `state` means no matching
transfer or wrong transfer kind. `incomplete` means bytes are still missing and
keeps the transfer open. `hash` means SHA-256 differs. `format` means bytes fail
media validation or the declared format is unsupported. `size` means a size or
decoded dimension exceeds a limit. `full` means insufficient filesystem space.
`storage` means unavailable storage or a persistence failure. `memory` means
allocation failed. `in-use` means a stored Layout or current live value references
the Asset. `layout`, `page`, `tile`, and `stream` mean the named target does not
match. `type` means a value type differs from the stream declaration. `asset`
means an image reference is absent or incompatible. `sequence` means an older
update or conflicting reuse of its counter. `rate` includes `retry_ms`, 1 to
1000, and means throttled. `idle` means photos are disabled or the Device is awake.

## Device name

`v2.name.get` has no other fields and returns `name`.
`v2.name.set` requires `name` and returns the stored `name`. The name is 1 to 48
UTF-8 bytes, contains at least one non-whitespace Unicode code point, and obeys
the string rules above. No truncation, normalization, or trimming occurs.
The factory default is `Open TouchDeck`. Firmware stores the name in NVS under
namespace `td_device`, key `name`, separately from Layouts and network pairing.
It acknowledges only after durable commit. An identical set causes no write.
A failure preserves the old name. Missing NVS data uses the default; corrupt
stored text is ignored. A normal Firmware update preserves the name.

## Assets and bulk transfers

An Asset id is the full SHA-256 of the exact encoded file, 64 lowercase hex
characters. The on-Device file layout is internal to Firmware. The format is
detected from the validated bytes, so no mutable sidecar is needed. Existing
16-digit `file:` icons and `/storage/icons` remain separate.

| Request fields after `m` | Successful response fields after `m,ok` |
|---|---|
| `v2.asset.stat`: none | `total_bytes`, `used_bytes`, `reserved_bytes`, `free_bytes` |
| `v2.asset.list`: `after` null or an Asset id | `items:[{id,bytes,format}]`, `next` null or an Asset id |
| `v2.asset.put`: `id`, `bytes`, `format` | `present`, `xfer` null or integer 1 to 255 |
| `v2.asset.end`: `xfer` | `id`, `bytes` |
| `v2.asset.delete`: `id` | `deleted` Boolean |

`free_bytes = max(0,total_bytes-used_bytes-reserved_bytes)`. Reservations include
open uploads and filesystem headroom, and cannot double-count already-written
temporary bytes. It is a snapshot; a subsequent upload may still return `full`.
Admission reserves the encoded size plus metadata headroom before acceptance.
No automatic eviction or formatting is permitted on a v2 write failure.

Listing returns at most four committed Assets, ordered by id, strictly after
`after`. `next` is the last returned id if more remain, otherwise null. `after`
may name a deleted id. Listing is not a snapshot across requests. Mutations may
require a fresh scan. Temporary files never appear. Delete of a missing id is
successful with `deleted:false`. Delete checks every stored orientation and
current live image reference under the same storage lock as Layout commit.
An unreadable stored Layout prevents deletion with `storage`.

Put of an existing, validated identical file returns `present:true,xfer:null`.
A conflicting declared size or format returns `format`. Otherwise it returns
`present:false,xfer:X`. One bulk transfer is open across v1 and v2 operations.
A new v2 put returns `busy` rather than replacing one. V1 put/get/update commands
keep their existing replacement rules, including aborting an open v2 transfer.
The Companion serializes bulk operations and never interleaves OTA with Assets
or photos. Only the initiating session can finish a transfer.

The Companion sends the existing chunk frames, starting at index 0. Every chunk
except the last has exactly 1021 data bytes. The last has the remaining positive
byte count. Window 4, cumulative acks, 200 ms retries, and five transmissions
remain v1. Duplicates and gaps receive the current `next`, without a second write.
A chunk with the wrong transfer id is ignored. Invalid chunk length leaves
`next` unchanged. Chunk frames introduce no JSON error response. A failed sink
aborts the transfer, stops acks, and records its reason for the end request.
An end on that failed id returns the recorded reason until the next bulk open.
No chunk for 2 seconds aborts a receiving transfer. Disconnect and selection
change also discard temporary data. Stale ids return `state`; ids are not reused
within 2 seconds of closure, and the Companion drains old replies before reuse.

`v2.asset.end` checks size, SHA-256, format, and all decoded media limits, then
flushes and atomically renames the temporary file. A retry of the most recently
successful end returns the same result until a new bulk transfer opens.
After reconnect or an uncertain result the Companion repeats put by hash.
This is idempotent upload, not persistent partial resume. Reset never publishes
a partial Asset; boot removes leftover temporary files.

`tdi1` is exactly [icon-format.md](../icon-format.md). Asset format `gif`
requires normalized GIF89a: one global palette of at most 256 colors, no local
palettes, full-canvas frames at 0,0, no interlace, disposal 0 or 1 only and at
most one transparent index. Every frame delay is at least `anim.min_delay_ms`.
NETSCAPE2.0 specifies an infinite loop. Other GIF encodings return `format`.
Canvas dimensions obey `anim.image`, frame count obeys `gif_frames` and the sum
of full-canvas pixels over all frames obeys `gif_pixels`.
JPEG requires baseline sequential 8-bit three-component JPEG,
no progressive or arithmetic coding, dimensions within `caps.image`. Firmware
validates complete decoding before commit and never trusts claimed dimensions.
The Companion resizes media before upload. Encoded byte limits do not replace
decoded memory limits. GIF playback runs on the Device.

## Layout references and live values

Layout schema 1 remains unchanged and rejects new fields. Schema 2 is reserved
for the extension defined here, implemented separately with its validator.
It retains schema 1 rules except `schema:2` and these optional Tile properties:
`asset`, null or `sha256:<64 lowercase hex>`; `render`, a supported Tile kind;
and `streams`, an object mapping stream ids to one of `number`, `series`, `text`,
or `image`. Missing `render` means `static`; missing `streams` means empty.
Stream ids match `^[a-z0-9_]{1,15}$`. `asset` and a non-null `icon` cannot coexist.
Static Assets use tdi1; gif Tiles require a GIF Asset. In this release JPEG
Assets serve image live values only. `bar` and `now-playing` reject a non-null
`asset`. Schema 2 also permits `input:true` only on a `bar` with a `widget`
action; omission means read-only, and explicit false is invalid.

| Renderer | Stream | Type | Required |
|---|---|---|---|
| graph | value | number or series | yes |
| bar | value | number, 0..1 | yes |
| bar | muted | number, nonzero means muted | no |
| now-playing | title | text | yes |
| now-playing | artist | text | yes |
| now-playing | cover | image | yes |
| now-playing | state | number: 0 idle, 1 paused, 2 playing | yes |

Unknown streams and wrong types are invalid. Static and gif Tiles declare no
streams. A graph number appends one sample up to `caps.series`; a series replaces
history. A bar value outside 0..1 returns `args`. Now-playing hides controls at
state 0. With nothing playing, publish all four streams: `state` 0, `title` "",
`artist` "" and `cover` null. The Device and preview draw "Nothing playing"
from copy.md and ignore title and artist while idle; no stream stays unset.
GSMTC Changing keeps the last state value. The golden frames include this idle
publish. Renderer geometry and hit regions are in the
[now-playing vectors](../live/now-playing-vectors.json) and
[bar vectors](../live/bar-vectors.json).

Every Page is built at activation: count image-bearing Tiles across the entire
Layout, including inactive Pages. Exceeding `caps.live_images` returns `size`
before commit. Repeated images still count once per Tile.

The existing `layout.put` and `xfer.end` carry schema 2 only after v2 negotiation
and advertisement in `layout_schemas`. They retain v1 framing, errors, CRC-32,
and atomic replacement behavior. Missing or incompatible Assets refuse the
Layout before commit. Firmware pins references in both stored orientations.
The `layout` field in a live request is the SHA-256 of the exact compact Layout
bytes last committed for the active orientation, or `builtin` for the built-in
Layout. This prevents a late update from applying to reused Page and Tile ids.
A successful Layout activation clears all live values and their counters.

`v2.live.set` requires `layout`, `page`, `tile`, `stream`, `n`, `ttl_ms`, `type`,
and `value`. Page and Tile ids retain schema 1 syntax. The Tile must belong to
that Page, including an inactive Page. `n` is an integer 1 to 4294967295,
strictly increasing per `(layout,page,tile,stream)` in the session. At exhaustion,
select v2 again. An identical retry with the same `n` returns success without
refreshing the stale deadline or consuming rate allowance. Conflicting reuse
or an older `n` returns `sequence`. Success returns `n`.

| `type` | `value` |
|---|---|
| `number` | A finite JSON number in the inclusive range -1e9 to 1e9 |
| `series` | 1 to `caps.series` numbers in that range, oldest first; replaces the full series |
| `text` | A string up to `caps.text_bytes` UTF-8 bytes, empty permitted |
| `image` | null to clear, or `sha256:<id>` for an existing JPEG Asset within `caps.image` |

`ttl_ms` is 1000 to 60000. Receipt uses the Device's monotonic clock; no clock
synchronization is needed. On expiry, Firmware retains the last value, draws it
at 60 percent opacity with an unavailable marker, and stops graph advancement.
An unset value shows an unavailable marker. Offline dimming and actions retain
v1 behavior. Stale values never become zero. A fresh accepted value clears stale
state. The renderer redraws dirty Tiles at most every 100 ms.

Each stream and the whole Device use independent one-second fixed windows from
session selection, capped by `live_hz` and `live_total_hz`. Over-limit updates
return `rate` without changing value, counter, or expiry. The Companion normally
sends at 1 Hz, coalesces queued updates per stream, and never builds an unbounded
queue. It limits serialized live requests to 256 bytes for the budget below;
larger valid values reduce the update count to retain the same byte budget.

Text size counts decoded UTF-8 bytes, not JSON escape bytes. Empty text is valid;
control characters follow the string rules above, and oversize text returns
`size`. The Companion cleans, folds and scalar-cuts using `fit_text` and
[text vectors](../live/text-vectors.json). The Device ellipsizes visually and
uses placeholder boxes for glyphs it lacks; there is no marquee.

An image reference is checked before committing value, counter or expiry.
Missing, incompatible or undecodable JPEG returns `asset`; allocation failure
returns `memory`. Decode once outside the live lock, swap on the UI task, and
retain the Asset reference until the UI releases it. An identical image heartbeat
requires no decode. Null clears the art immediately. On track change send null
with the new title; never retain the previous track's cover beside new metadata.

The media profile uses `text_bytes:128`, `streams:4`, `formats:["tdi1","jpeg"]`,
`image:[256,256]`, `asset_bytes:32768`, and `live_images:4`. With GIF enabled,
retain its larger `asset_bytes:262144`, add `gif` and its `anim` caps. The
production Firmware implements the media profile and draws every Tile kind it
advertises.

## Touch input events

A selected v2 session may add `part` to `press`. It matches `^[a-z_]{1,15}$`:
`previous`, `play_pause`, `next` for now-playing, or `mute` for a bar declaring
`muted`. A tap outside a part sends no part. Compact now-playing Tiles use the
whole Tile for play/pause. Idle now-playing has no active control zones.

`v2.input` is a type 1 unsolicited frame, flags 2, frame sequence 0:

```json
{"m":"v2.input","page":"media","tile":"volume","stream":"value","phase":"move","value":0.4213,"seq":118,"t":5234112}
```

`phase` is `move` or `end`; `value` is finite, 0..1, rounded to four decimals.
`seq` is a rising u32 since boot, separate from press counters; `t` is boot ms.
Only a bar with `input:true` sends it. Moves are latest-only snapshots at most
every 50 ms, followed by an immediate end replacing any pending move. No ack
or replay is required. A tap on the track does nothing: the gesture must pass
pager slop. Press-lost ends it; session/Layout change or offline cancels it.
Stale or unset values disable input. During dragging the Device draws locally;
after lift it settles until live readback is within 0.01 or 1.5 s elapses, then
draws the newest live value. Mute is never predicted. A set above zero unmutes.

## Page on screen

`v2.page` is a type 1 unsolicited frame, flags 2, frame sequence 0, that names
the Page on screen by its schema 1 id:

```json
{"m":"v2.page","page":"media"}
```

Firmware that reports Pages sends it once right after each successful protocol
2 selection, and again whenever the Page on screen changes (a swipe, a page
action, the first touch waking to the last Page) or a Layout loads, which starts
on its first Page. It is sent only while v2 is selected and the link is online,
and never on v1. A repeat of the last report can occur and means nothing new.
No ack or replay is required, so a lost event leaves the Companion with the
previous Page until the next change or session. There is no hello field or
capability for it: a Companion treats the Page as unknown until the first event
of a session, so Firmware without `v2.page` simply never reports one. The id is
resolved against the Layout the Companion read from the Device; an id the read
Layout lacks means unknown.

## Photo frame, idle, and clock

A picture is one logical message, `v2.photo.put`, followed by its chunks and
`v2.photo.end`. It is exactly one complete JPEG, not screen-frame streaming or
base64 inside JSON. Put requires `bytes` and `sha256`; success returns `xfer`.
End requires `xfer`; success returns `shown:true`. Transfer rules are those for
Assets, but storage is bounded RAM and nothing is written to LittleFS. Size is
1 to `caps.photo_bytes`, at most 128 KiB. JPEG format follows the Asset rules;
dimensions are at most `caps.screen`, not `caps.image`. The Device keeps at most the current picture and one incoming
picture. Decode failure keeps the old picture or clock. Success replaces the
picture atomically only if the Device is still idle. Touch cancels a pending
photo and its end returns `idle` until the next bulk open.

`v2.idle.get` returns `enabled`, `timeout_s`, `interval_s`, and
`fallback:"clock"`. `v2.idle.set` requires all four fields and returns the same
fields after durable NVS commit under namespace `td_device`, key `idle`.
Defaults are false, 60, 15, and clock. `timeout_s` is 10 to 86400 and
`interval_s` is 5 to 3600. These settings govern the photo-frame idle mode,
independently of v1 `settings.put` and its existing power behavior. When enabled,
photo idle mode keeps the screen awake at the configured brightness; disabling
it immediately exits photo mode and restores v1 power behavior. Unsupported
photo-frame or clock kind returns `unsupported` on enable.

Idle starts after no touch for `timeout_s`. The Companion may send one picture
per `interval_s`, measured between successful photo commits, only while idle.
The first picture after entry is allowed immediately. Early put returns `rate`
with remaining milliseconds in `retry_ms` (for photos this may exceed 1000).
A put while awake returns `idle`; the Companion retries no faster than once a
second. No new event or polling message is needed. A pending upload must keep
chunks flowing within the 2-second transfer timeout.

The first touch wakes to the last active Page, cancels the incoming picture,
and is consumed, so it cannot trigger an action. Offline or no picture for two
configured intervals shows the clock. Going offline switches to the clock
immediately. The clock does not depend on the Companion rendering pixels.
`v2.clock.set` requires `unix_s`, integer 0 to 4102444799, and `offset_min`,
integer -840 to 840, and returns those two fields. Firmware advances this
volatile clock by monotonic elapsed time and applies the offset for display.
The Companion refreshes it on connection and time-zone changes. Unsynchronized
after boot, the clock displays `--:--`, never a guessed time. Clock and idle
settings never wake a sleeping Device by themselves.

## Throughput and scheduling budget

[ADR 0004](../../docs/adr/0004-firmware-draws-live-tiles.md) estimates about one
second for a 1024 by 600 RGB565 frame: 1,228,800 bytes. That is an architectural
estimate, not measured sustained upload throughput. [Firmware notes](../../docs/firmware.md)
record roughly 60 ms framebuffer copies and a 256-byte USB receive ring that can
lose packets if RX cannot run. They contain no sustained USB Asset benchmark.
No throughput measurement is claimed here. Issue #80 must measure simultaneous
uploads, live values, and tap latency before advertising these ceilings.

The following admission budget assumes a deliberately lower **100,000 wire
bytes/s**, aggregate both directions, and reserves 20,000 for taps, key reports,
pings, replies, console traffic, and retransmissions. Values below are estimates.
Every bulk frame yields to queued control writes. At most four chunks are ahead
of acks; RX and touch handling must run during decode and flash operations.

| Traffic | Estimated aggregate wire bytes/s |
|---|---:|
| 20 live updates/s, 256-byte JSON request plus 48-byte reply, 20 bytes framing and 4 bytes escape allowance | 6560 |
| One 128 KiB photo every 5 s, worst-case SLIP and ack overhead rounded to 2.1 times encoded size | 55051 |
| Asset upload at 8000 encoded bytes/s, same 2.1 multiplier | 16800 |
| Control and retry reserve | 20000 |
| Total | 98411 |

Photos and Assets share the single bulk slot, so the table is an average
schedule, not two concurrent transfers. At 128 KiB per five seconds a photo
uses about 3.7 seconds of the 73,440 wire bytes/s left after live traffic and
control reserve. The remaining time admits about 42 KiB of Assets per period.
A 1 MiB Asset needs about 131 seconds at 8,000 encoded bytes/s; without photos,
roughly 30 seconds at 34,000 encoded bytes/s. At the default 15-second photo
interval more time is available for Assets. The scheduler must defer a photo
or Asset, never evict the current transfer to meet the schedule.

A worst-case 2061-byte frame occupies about 20.6 ms at the assumed rate. Four
frames occupy about 82.4 ms, below the 200 ms retry deadline. Writers service
control queues between frames, not only at the end of a four-chunk batch.
The 20,000-byte reserve covers, for example, ten taps/s with three 128-byte
messages each, leaving 16,160 bytes/s for other control and retries.
A measured rate below the assumption makes the Companion lower Asset rate,
extend the photo interval, and coalesce live updates. Sustained failures pause
bulk traffic. Queue delay, JPEG decode time, flash latency, and real tap latency
remain implementation measurements; arithmetic does not prove UI responsiveness.

## Compatibility table

| Operation | V1 Firmware behavior | Companion with v1 Device |
|---|---|---|
| Plain hello | Returns `proto:1`, no `protocols` | Selects v1 only |
| Hello with `proto:2` | Ignores extra field, still returns 1 | Never sends absent advertisement; never treats 1 as v2 |
| `v2.name.get`, `v2.name.set` | `ok:false,error:"unknown message"` | Does not send; any local label is not persisted on Device |
| `v2.asset.stat`, `.list`, `.put`, `.end`, `.delete` | Same unknown-message error for each full name | Uses existing icons and store operations only |
| `v2.live.set` | Unknown-message error | Uses existing widget/dashboard messages only for compatible Tiles |
| `v2.photo.put`, `v2.photo.end` | Unknown-message error | Disables photo streaming |
| `v2.idle.get`, `v2.idle.set`, `v2.clock.set` | Unknown-message error | Uses v1 settings only |
| Schema 2 in `layout.put` | Transfer can open; validation at `xfer.end` rejects schema 2 | Never sends; keeps compatible schema 1 Layout |
| Chunks and acks | Existing transfer engine may consume them | Never sends v2 data unless a negotiated put succeeded |
| `v2.input`, `press.part` | Never emitted | Never inferred from v1 presses |
| Existing messages and events | Unchanged | Same names, framing, retries, and errors |

### Media extension compatibility

| Pairing | Device side | Companion side |
|---|---|---|
| Older v2 Device with new Companion | Does not advertise media caps/kinds or emit media input | Shows Needs Firmware; retains a compatible Layout |
| Media-capable Device with old Companion | Emits optional `press.part` and v2-only `v2.input` on a selected v2 session | Ignores the unknown field and event; does not route controls |

## Golden frames and verification

[golden-frames-v2.json](golden-frames-v2.json) uses the same `protocol`, `crc`,
`limits`, `frames`, and `streams` structure as the v1 fixture. `protocol:1` is the
frame version; `application_protocol:2` identifies this contract. `valid` means
valid framing, including error responses, not successful message dispatch.
The payload bytes in `decoded.payload_hex` are authoritative. Fixture names
identify direction and case. Optional `message` is a readable copy for review.

Fixtures cover discovery, selected hello, every new request and response,
each live value type, all error reasons, Asset and photo chunks and acks,
duplicate chunks, escaped bytes, and mixed tap traffic. They are codec fixtures,
not a claim that v2 handlers exist. The Rust test constructs frames from decoded
fields and checks exact wire bytes, then decodes fragmented streams with the
existing td-core codec. From `app/`, run `cargo test -p td-core --test golden_v2`.

## Animated GIF admission

GIF is off when `anim` is absent, even if `formats` lists `gif`. It also needs
`gif` in `tile_kinds` and nonzero `gif_frames` and `gif_pixels`. `image` continues
to bound static images; `anim.image` bounds animated canvases. `min_delay_ms`
is the minimum duration of each frame, not the live-value update rate.
`page_tiles` counts animated Tiles on each Page; `page_pixels` sums their canvas
areas, counting repeated references separately. Every Page must fit both limits.

Proposed caps, pending the combined board proof: `asset_bytes:262144`,
`anim:{image:[128,128],min_delay_ms:100,page_tiles:4,page_pixels:65536}`,
`gif_frames:60` and `gif_pixels:983040`. Production Firmware keeps
`formats:["tdi1"]`, `gif_frames:0` and no `anim`. `CONFIG_TD_GIF_CAPS` defaults
off and is a contract-test switch only until validation and playback pass proof.
Golden hello vectors cover both states and the 1024-byte bound.

Schema 2 accepts `render:"gif"` only with a non-null `asset:"sha256:<id>"`
and `icon:null`. The referenced Asset must have format `gif`. Missing `render`
means static. Schema 1 refuses `render`. The shared validators also accept the graph, bar and now-playing contracts;
production capability advertisement still requires implementation and proof.

Both sides enforce limits. The Companion refuses early; Firmware has the final
say on encoded bytes, complete decoding, memory, storage and per-Page budgets,
including existing/present Assets. Encoded bytes, dimensions, frame count,
cumulative decoded pixels and Page budgets use wire reason `size`; invalid
normalization, delay or loop encoding uses `format`; allocation failure uses
`memory`; insufficient storage uses `full`. GIF cadence never uses `rate`.
Failures preserve the committed Asset and Layout. Asset link failures retain
the typed reason through the API rather than becoming generic link text.
