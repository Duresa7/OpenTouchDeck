# Open TouchDeck network link v2

net-v2 carries link frames between a Device and the Companion over two
**Carriers** other than USB: TCP over the Device's Wi-Fi station interface, and
a BLE GATT service. Both Carriers move the same items in the same format. On
top of them the Companion runs the same link session as over USB, including
[link-v2](link-v2.md) protocol selection.

net-v2 replaces [net-v1](net-v1.md), which never shipped in Firmware. It keeps
net-v1's SRP-6a pairing with a six-digit code, the long-term keys, the busy
rule and the Device identity. It changes these things:

* Every session adds an ephemeral X25519 exchange, so a stolen long-term key
  doesn't decrypt recorded sessions ([ADR 0009](../../docs/adr/0009-net-sessions-add-x25519-for-forward-secrecy.md)).
* Records carry link frames whose application protocol is chosen by link-v2's
  hello negotiation, so link-v2 is no longer USB-only.
* One item format rides on TCP and on BLE GATT, with fragmentation rules that
  work at the smallest ATT MTU.
* Pairing windows open only from the Device, and failed codes back off across
  windows and reboots.
* A network session may run only the messages listed for it. Firmware updates
  need approval on the Device, and Wi-Fi credentials, opening pairing windows
  and removing Pairings stay USB-only.

The suite is fixed, with no negotiation: SRP-6a over RFC 5054's 3072-bit group
with generator 5 and SHA-512 for pairing; X25519 (RFC 7748) for the session
exchange; HMAC-SHA256 and HKDF-SHA256 for proofs and keys; AES-256-GCM with a
16-byte tag for records.

MUST, MUST NOT and SHOULD state requirements. All test values in
[net-v2-vectors.json](net-v2-vectors.json) are public synthetic fixtures. They
MUST NOT be used by a Device or a Companion outside the reference tests.

## Carriers and discovery

The Device's only radios are 2.4 GHz Wi-Fi and BLE. It has no Ethernet, so the
TCP Carrier always rides on Wi-Fi. A test harness may run the Firmware's network
service over an emulated network interface; that is a test stand-in, not a
Carrier.

A **carrier connection** is one TCP connection, or one BLE connection from the
moment the Companion has enabled notifications on both `tx` and `flow` (see
below). Each carrier connection carries at most one pairing exchange or one
session. Ending the session ends the carrier connection: TCP closes, and on BLE
either side disconnects. A new session always needs a new carrier connection.

**TCP.** The Device listens on TCP port 47820 on its station interface. The
Companion discovers it with DNS-SD over multicast DNS (RFC 6762 and RFC 6763),
service type `_touchdeck._tcp`, or connects to an address the person typed.
A typed address is an IPv4 address, an IPv6 address or a host name, with an
optional port that defaults to 47820. Discovery uses only standard DNS-SD and
no OS-specific service, so it works the same on Windows and Linux. Networks
that block multicast need the typed address. What the Device advertises is in
[Discovery privacy](#discovery-privacy) below: outside pairing mode, nothing
in it identifies the Device.
Both sides SHOULD disable Nagle's algorithm (`TCP_NODELAY`).

**BLE.** The Device is a GATT server and the Companion is the client. The
service and its characteristics use these 128-bit UUIDs:

| Name | UUID | Properties | Direction |
|---|---|---|---|
| service | `6198f058-afef-4469-b47f-07315672e2dd` | primary service | |
| `rx` | `6198f059-afef-4469-b47f-07315672e2dd` | Write Without Response | Companion to Device |
| `tx` | `6198f05a-afef-4469-b47f-07315672e2dd` | Notify | Device to Companion |
| `flow` | `6198f05b-afef-4469-b47f-07315672e2dd` | Notify | Device to Companion |

The Device advertises the service UUID in its advertising data while it can
accept a carrier connection. Its scan response carries service data for the
same UUID, using the compact format below. The Companion MUST NOT identify a
Device by its BLE address. Both Windows and Linux expose service data by UUID.
No Device may ship the BLE Carrier until its radio integration follows the
pairing-mode and rotation rules of [Discovery privacy](#discovery-privacy).

The link service needs no BLE pairing, bonding or link-layer encryption, and
the Companion MUST NOT start BLE pairing for it. net-v2 authenticates and
encrypts everything above GATT. Keeping the OS out of the trust decision means
no OS-level bond that reconnects on its own at boot, and the same Pairing works
on Wi-Fi and BLE. Bonds that the Device's Bluetooth keyboard role makes with
its hosts are separate and outside this document.

Advertised data and TXT records are untrusted hints. The Companion pins
`device_id` when it pairs and checks it in every session. An address change
doesn't change the Pairing.

### Discovery privacy

A stable value in an advertisement lets anyone on the LAN follow a Device from
network to network. So the Device advertises its id, its name and its Pairing
count only in **pairing mode**, while a pairing window is open and a person is
pairing on purpose. At all other times it advertises a fresh random nonce and
one short **hint** per Pairing, which only the paired Companion can recognise.

Each Pairing has a discovery key, derived from its LTK on both sides, so
pairing sends nothing new and neither side stores anything new:

```text
K_disc = derive(LTK, "discovery")
hint = the first 8 bytes of MAC(K_disc, "hint" || nonce)
```

`derive` is the HKDF-Expand defined under
[Transcript, proofs and key delivery](#transcript-proofs-and-key-delivery),
here with the LTK as its key. The LTK is 32 uniformly random bytes, so it
needs no extract step. A Pairing's hints stop the moment it is removed.

**The TXT record.** The record of `_touchdeck._tcp` has these keys:

| Key | When | Value |
|---|---|---|
| `proto` | always | `2` |
| `n` | always | `nonce`, 16 random bytes as 32 lowercase hex digits |
| `h` | always | Four 8-byte hints, concatenated as 64 lowercase hex digits |
| `id` | pairing mode only | Device id, 12 lowercase hex digits |
| `name` | pairing mode only | The Device name from link-v2, at most 48 UTF-8 bytes |
| `paired` | pairing mode only | Number of stored Pairings, decimal |

`h` always holds four hints: one for each stored Pairing, in slot order, and
8 random bytes for each free slot. A random filler looks like a hint, so the
record doesn't say how many Pairings there are. The service instance name is
`TouchDeck ` followed by `label`, and the host name is `td-` followed by
`label`, where `label` is the first 4 bytes of `nonce` as 8 lowercase hex
digits. Outside pairing mode, nothing else in the record, the instance name or
the host name may carry the Device id, the factory MAC, the Device name, the
Pairing count or any other value that outlives one nonce.

**Rotation.** The Device draws a new nonce, new fillers and so a new label at
boot, every 900000 ms of uptime, when a pairing window opens or closes, and
when a Pairing is added or removed. Each change withdraws the old record and
announces the new one. The payload supplies a linkable value within one nonce,
at most 15 minutes. Timing and network addresses can still correlate observations
across rotations. A shorter period would churn mDNS
caches for little gain, because a Companion browses only when the person asks.

**Why a nonce and not the time.** A time-window hint needs the Device and the
Companion to agree on the time, and the Device has no battery-backed clock
and may never reach a time server. A nonce needs no clock at all: the
Companion computes the hint for the nonce it sees, so clock skew can't make a
paired Companion miss its Device. The nonce has 128 bits, so it doesn't repeat
and an observer can't build a table of hints.

**Why 8 bytes.** A hint only chooses which address to try. A false match costs
one failed session, because a session still needs the LTK. With 64 bits, a
Companion with four Pairings comparing every hint of 32 records has a chance
below 2^-54 of one false match. Four hints and a nonce keep the record near 110
bytes, well inside one mDNS packet.

**The Companion.** For each record it reads, the Companion computes the hint
for each of its Pairings with that record's `n`, and compares it with each of
the four hints in `h` in constant time. A match means that Pairing's Device
answered at the record's address. The Companion tries a session there, and
keeps the address that last worked until a session at the new one succeeds.
Replaying an old record from another address therefore only costs a bounded
number of failed tries: after two consecutive `busy` replies there, the
Companion drops the trial and immediately tries the address that last worked.
A later discovery may set a new trial. A record that matches no Pairing and has no `id` is someone else's
Device, and the Companion drops it. A record with `id` is a Device in pairing
mode, which the Companion may list for pairing. A record without `n` and `h`
is malformed and dropped.

**The BLE scan response.** BLE uses the same discovery key with its own,
smaller layout, because a legacy scan response holds 31 bytes and the service
data's AD header and 128-bit UUID take 18 of them. The service data after the
UUID is exactly 13 bytes:

| Mode | Service data |
|---|---|
| outside pairing mode | `0x02 \|\| nonce[6] \|\| hint[6]` |
| pairing mode | `0x82 \|\| device_id[6] \|\| paired:u8 \|\| padding[5]` |

```text
ble_hint = the first 6 bytes of MAC(K_disc, "ble-hint" || nonce)
```

The label `ble-hint` keeps BLE hints apart from TXT hints, and the BLE nonce is
drawn independently of the TXT nonce, so an observer of both radios can't link
them through the payload. The Device draws one BLE nonce and four random
six-byte fillers at the same moments as the TXT nonce: at boot, every 900000 ms
of uptime, when a pairing window opens or closes, and when a Pairing is added
or removed. Slot `i` carries Pairing `i`'s hint, or filler `i` when the slot is
free. The scan response shows one slot at a time and moves to the next slot
every 1000 ms, cycling all four whatever the Pairing count, so neither the
cadence nor the payloads say how many Pairings there are. No slot number is
sent. Pairing mode carries no name and no nonce; its padding is fresh random
bytes at each rotation. `paired` is at most 4.

Two rules belong to the radio code that drives this builder:

* Each rotation replaces the scan response and the Device's resolvable or
  non-resolvable private BLE address together. A fixed address would link
  every nonce, and a new address with an old payload would link the
  addresses.
* Closing a pairing window removes the pairing-mode payload at once.

The Bluetooth keyboard role advertises separately, under its own rules; it
doesn't add its name or any other stable value to these advertisements.
Its own advertising address is still public; see
[What stays visible](#discovery-privacy) below.

**Why 15 minutes on BLE too.** One period for both Carriers keeps one rotation
rule in the Firmware, and the linkable window is the same whichever radio an
observer listens to. A shorter period would buy little: a person who sits near
the Device for longer than a period can see it anyway.

**Why a 6-byte nonce and a 6-byte hint.** They are what fits beside the
marker, and they keep BLE discovery on legacy advertising, which every
Bluetooth adapter supports, instead of requiring extended advertising. The
cost is in two places, both acceptable:

* A repeated nonce lets an observer link the two periods it appears in. Over
  ten years of uninterrupted 15-minute rotations a Device draws about 350000
  nonces, and the chance that any two of them are equal is about 2.2 * 10^-4.
  Reboots and Pairing changes add draws and raise this slightly.
* A Companion with four Pairings that sees 32 Devices with four slots each
  makes 512 comparisons per scan, so the chance of any false match is at most
  512 / 2^48 = 2^-39. A false match costs one failed session.

A hint chooses which BLE address to try and is never authentication; anyone
can replay one. The session still proves the LTK and pins `device_id`.

**The Companion on BLE.** It scans actively for at least 5000 ms, so it sees
every slot, and handles changed service data even from an address it has
already seen. For each payload it computes the BLE hint for each of its
Pairings with that payload's nonce and compares it with the payload's hint in
constant time, checking every Pairing, and it merges matches by Pairing. A
matching Device answered at that address for now; the Companion never pins a
BLE address. A pairing-mode payload is a Device the Companion may list for
pairing. A payload with any other length or marker, or with `paired` above 4,
is dropped. In particular, the earlier draft's `proto || device_id || paired`
payload is never read, so neither side falls back to a stable id outside
pairing mode.

**Existing Pairings.** No migration step is needed. `K_disc` comes from the
LTK, which every Pairing already stores, so the Device and the Companion
derive it when they load a Pairing, whenever it was made. Nothing is added to
NVS or the credential store, and nobody re-pairs. The Companion keeps derived
keys in memory only and never logs or serializes them. Firmware from before
this rule, which advertised its id at all times, must be updated: the
Companion no longer reads that format.

**What exists today.** The Firmware's `td_netlink_advert_build` and
`td_netlink_ble_advert_build` build both formats, and td-link's
`net::mdns::Advert::matches` and `net::ble_discovery::recognise` match them.
The Firmware's mDNS publisher comes with its network service, and the BLE
net-v2 advertiser and scanner, with the rotation timers and address changes
above, come with the BLE Carrier.

**Passive privacy only.** Someone who records a valid `session.open` can
send it to a Device later: the paired Device answers with a challenge, while
other Devices answer `auth`. This distinguishes that Device even across
advertisement nonce rotations. Preventing this active probe would require
binding the tag to fresh Device context, which this protocol does not do.

**What stays visible.** The Device's Wi-Fi station address is its factory MAC,
which is `device_id` itself on the ESP32-S3. Anyone in radio range sees it in
every frame, whatever mDNS says. Hiding it needs a random station address,
which this document doesn't yet require. The TCP port, the service type and
the fact that a TouchDeck is present stay visible too.

The Bluetooth keyboard role's address is visible on BLE in the same way. While
no bonded host is connected, the keyboard role advertises with no time limit
from the Device's public Bluetooth address, which the ESP32-S3 derives from
the factory MAC (`device_id` + 2). Anyone in radio range can follow the Device
by that address, whatever the net-v2 discovery advertisements carry, so BLE
does not yet meet the rule that nothing stable is advertised outside pairing
mode. The address is still public because the keyboard role was built and
bonded with its hosts before this rule existed. Moving it to a resolvable
private address changes how the Device bonds with its hosts, so that change
is tracked separately in
[#401](https://github.com/Duresa7/OpenTouchDeck/issues/401).

## Device identity

`device_id` is the six-byte factory base MAC from eFuse, in byte order. It is
the link-v2 `id` (12 lowercase hex digits) and the USB serial number. It is not
the BLE address, the Wi-Fi interface address if that ever differs, or any
randomized address. The same Device has the same `device_id` on every Carrier,
so the Companion keeps one Device record and changes only its active Carrier.

A selected link-v2 hello over a net-v2 session MUST carry an `id` equal to the
session's `device_id`. The Companion closes the session otherwise. The id is
identity, not authentication; the session handshake is the authentication.

## Ownership and the busy rule

The Device admits one net-v2 **owner** across both network Carriers: a carrier
connection that starts while there is no owner, until it closes. A connection
bound to a pairing window is an owner. While an owner exists, any other carrier
connection, on TCP or on BLE, receives the `busy` error as soon as it starts
and then closes. The Device writes that error within 100 ms and allocates no
SRP or X25519 state for it. One exception lets a real Companion in past a
connection that is only holding the Device:

* **Takeover.** An owner is *unproven* until its first handshake item has
  arrived and, for a session, until the Device has verified its
  `client_proof`. A new carrier connection whose first item is `session.open`
  with an `open_tag` that matches a stored Pairing takes over from an
  unproven owner that is not bound to a pairing window. The Device closes the
  old connection without an error and continues with the new one. A
  `session.open` whose tag matches no Pairing gets `auth` and takes nothing
  over. An owner bound to a pairing window is never taken over; the person at
  the Device can cancel it.

Unauthenticated connections are kept short:

* The first handshake item MUST arrive within 2000 ms of the carrier
  connection starting, or the Device closes it with `expired`.
* On BLE, the Companion MUST enable both notifications within 5000 ms of the LE
  connection, or the Device disconnects. An LE connection that hasn't started a
  carrier connection is not an owner, but it does use a connection slot.
* **Per-peer backoff.** A peer is a TCP source address or a BLE address. The
  Device counts a **failure** against a peer only when that peer's carrier
  connection, or its BLE LE connection that never started one, ends for one
  of these reasons, all caused by the peer:
  * a missed deadline: the first handshake item (2000 ms), both BLE
    notifications (5000 ms), session setup (10000 ms) or a pairing attempt
    (10000 ms);
  * `auth`, which includes an `open_tag` that matches no Pairing and the failed pairing
    proof that ends the connection;
  * a bad proof: a `client_proof` or `commit_proof` that fails its check;
  * a malformed item: anything this document says closes the carrier
    connection for its bytes, including `version`, `args`, a wrong length,
    kind, field or state transition, invalid UTF-8 or a refused name, an
    invalid public value or an all-zero `Z`, a broken fragment rule, a
    prepared write or write with response, a write beyond the credit or
    before the carrier connection starts, and disabling notifications;
  * a close the peer starts before the Device has sent a terminal error or
    decided to close: a TCP FIN or RST, or a BLE disconnect whose reason is
    one a peer sends when it terminates, 0x13 (remote user terminated),
    0x14 (low resources) or 0x15 (power off). Otherwise a peer could
    connect, idle until just before the first-item deadline, drop and
    repeat for free.

  Nothing else counts. In particular these never count against the peer
  whose connection ends: a connection that another `session.open` takes over,
  a `busy` refusal (including one sent because of this backoff), and a
  `session.open` refused because USB is online. Neither do `closed`, `full`,
  `cancelled`, `storage`, an expired pairing window, a committed Pairing, a
  peer closing after the Device's terminal error, or a radio loss: any other
  BLE disconnect reason the Device didn't cause, such as supervision timeout
  (0x08). A modified controller can report any reason, so, like the address,
  this limits nuisance rather than a determined attacker.

  Each failure refuses that peer for a delay from the failure that starts at
  1000 ms, doubles on each further failure up to 60000 ms, and resets when
  that peer reaches `session.ready`. The Device applies the refusal to the
  peer's first handshake item, as it does a takeover: a `session.open` whose
  `open_tag` matches a stored Pairing skips the backoff and goes on to the
  handshake, and any other first item gets `busy`. So someone who sends from
  the real Companion's public BLE address with a made-up tag, which gets
  `auth` and counts against that address, can't keep the real Companion
  out. The Device keeps at least eight peers, evicting the one whose last
  failure is oldest. A tag needs the LTK, so only a replay of a whole
  `session.open` that someone recorded passes the check, and the replayer
  can't finish the handshake. Because takeovers and refusals don't count,
  such a replay can delay the real Companion's handshake but can't push its
  address into backoff.
  Addresses are easy to change, so this limits nuisance, not guessing; the
  pairing rules below do that.

USB wins. While the USB link is online (a valid `hello` within the last
5000 ms, as in link-v1), the Device:

* answers `session.open` with `busy`,
* closes an established net-v2 session when USB comes online,
* still allows pairing, because pairing carries no link frames. A person can
  set up Wi-Fi, open a pairing window and pair while the Device is plugged in.

USB itself is never refused. Replies return on the Carrier that carried the
request. Events go to the online Carrier. Because a session and an online USB
link never overlap, there is at most one. A link-v2 transfer belongs to the
Carrier that started it and is aborted when that Carrier closes. link-v2
protocol selection is per session: a new net-v2 session starts at protocol 1,
independent of USB.

A Companion uses one Carrier per Device and prefers USB when USB answers
`hello`. Switching Carrier closes the old session, releases held keys, and
starts a new `hello` on the new Carrier.

## Items

`||` concatenates byte strings. `u16be` and `u64be` are unsigned big-endian
integers. `H` is SHA-256, `H512` is SHA-512 and `MAC(k,m)` is HMAC-SHA256.
Literal strings are ASCII with no terminator unless `\0` is written. `hex` is
lowercase. Random values come from a cryptographic RNG and are each generated
independently. Comparisons of proofs and tags MUST be constant-time. Secrets
MUST be erased when their state ends.

Each direction of a carrier connection is a byte stream of **items**:

```text
item = u16be(len) || body
```

`len` excludes its own two bytes. A receiver reads exactly `len` bytes, and
rejects a length outside the bounds of its current state before it allocates.
There are two phases, and receivers MUST NOT tell them apart by sniffing bytes.

* Before `session.ready`, `body = kind:u8 || version:u8 || fields`. `version`
  is 2. Handshake bodies are at most 512 bytes. No link frame travels in this
  phase.
* After `session.ready`, `body = ciphertext || tag`, an AEAD record. Its
  plaintext is exactly one complete link frame body: the six-byte header,
  payload and two-byte CRC, as in link-v1, with frame version 1 and a
  little-endian payload length. Plaintext is 8 to 1032 bytes, so `len` is 24
  to 1048 and an item is at most 1050 bytes.

No text console, log output, SLIP delimiter or SLIP escaping runs on a network
Carrier. The CRC stays inside the record so every Carrier hands the link
session the same frame bodies.

### TCP

Items follow each other on the TCP stream. TCP reads may split or combine
items.

### BLE fragments

On BLE the Companion-to-Device stream is the sequence of `rx` writes, and the
Device-to-Companion stream is the sequence of `tx` notifications. Each write or
notification is one **fragment**:

* A fragment holds 1 to `F` bytes of exactly one item. An item's bytes go out
  in order, as `ceil(len_item / F)` fragments. Every fragment but the item's
  last has exactly `F` bytes. The next item starts a new fragment.
* `F = ATT_MTU - 3`. A sender uses the negotiated ATT_MTU only when its BLE
  stack reports it. Otherwise it uses the ATT minimum, 23, so `F = 20`. The
  Device always knows its ATT_MTU. A Companion on WinRT can read it from the
  GATT session; one on BlueZ through btleplug may not be told, and then sends
  20-byte fragments. Neither side requires a larger MTU.
* A receiver accepts fragments up to its own `ATT_MTU - 3`. One that doesn't
  know its ATT_MTU accepts up to 514 bytes, the most an ATT_MTU of 517
  allows, because the MTU exchange may have succeeded below a stack that
  doesn't report it. So a Companion with an unknown MTU sends 20-byte
  fragments and still receives the Device's 244-byte notifications.
* Each sender fixes its `F` when the carrier connection starts and keeps it
  for that connection, even if it learns a larger MTU later.
* Because `F` is at least 20, an item's first fragment always holds its whole
  length prefix.
* A receiver doesn't know the sender's `F` in advance. It learns it from the
  first fragment that isn't an item's last, and from then on checks it. It
  closes the carrier connection on:
  * an empty fragment, or one longer than its own `ATT_MTU - 3`, or 514 when
    it doesn't know its ATT_MTU;
  * a first fragment shorter than 2 bytes;
  * a fragment that isn't its item's last and is shorter than 20 bytes or
    differs from the learned `F`;
  * a last fragment longer than the learned `F`;
  * a fragment that runs past the end of its item;
  * a length outside the current phase's bounds: 3 to 512 for a handshake
    item, 24 to 1048 for a record.
* Prepared (long) writes and writes with response to `rx` are refused with an
  ATT error, and the Device disconnects.

A 1050-byte item is 53 fragments at `F = 20` and 5 at `F = 244`. The
`ble_fragments` in the vectors show both for one record each way.

**Flow control.** Writes without response have no acknowledgement, so the
Device grants credit on `flow`. The value of each `flow` notification is
`u64be(limit)`: the total number of `rx` bytes the Companion may have written
since the carrier connection started. The limit never decreases.

1. The Companion connects, optionally exchanges MTU, and enables notifications
   on `flow` and on `tx`, in either order. The carrier connection starts when
   both are enabled.
2. The Device then notifies `flow` with its initial limit, at least 1050 so one
   maximum item fits. 2100 is RECOMMENDED. The Companion writes nothing to
   `rx` before that notification.
3. The Companion keeps `bytes_written <= limit`. The Device raises the limit as
   it frees receive buffer space, in steps it chooses.
4. A write beyond the limit, a write before the carrier connection starts, or
   notifications disabled on `tx` or `flow` ends the carrier connection.

`tx` notifications have no credit. The Device waits for its stack's buffers
and never drops a notification. Lost, duplicated or reordered fragments fail
the record's length, tag or counter check, so the session closes rather than
accepting corrupt data.

**Connection parameters.** The Device SHOULD request a connection interval of
15 to 30 ms, peripheral latency 0, and a supervision timeout of 4000 ms. A
Companion SHOULD write several fragments per connection event when its stack
allows. At one 20-byte fragment per 30 ms event, the slowest case these rules
allow, a 1050-byte item takes about 1.6 s, within the 5000 ms link expiry.
These are estimates, not measurements. BLE throughput is far below USB, so the
Companion lowers Asset rate, photo interval and live update rate as link-v2's
throughput section describes.

## Handshake errors

Errors are handshake items with kind `0x7f`, version 2 and a one-byte reason.
They are unauthenticated hints. They never establish identity or cause a stored
Pairing to be deleted. Unknown kinds, versions, trailing bytes, unexpected
state transitions, malformed UTF-8 and wrong field lengths close the carrier
connection. A Device that receives an item with version 1 sends `version`.

| Reason | Name | Meaning |
|---|---|---|
| 1 | `busy` | Another owner holds the Device, or USB is online and this is `session.open` |
| 2 | `version` | Unsupported item version. The Companion never falls back (see below) |
| 3 | `args` | Invalid handshake fields or state |
| 4 | `full` | Four Pairings exist, or this Companion id already exists. A slot frees only when a Pairing is removed (see Pairing slots) |
| 5 | `auth` | Authentication failed, including an unknown Companion id |
| 6 | `expired` | A deadline or the pairing window expired |
| 7 | `cancelled` | The person cancelled pairing on the Device |
| 8 | `storage` | The Pairing could not be committed |
| 9 | `closed` | No pairing window is open on the Device, or pairing is locked out |

For example, `busy` is hex `00037f0201`. **No fallback after `version`.** An error item is unauthenticated, so anyone on
the path can forge one. A Companion MUST NOT answer `version`, or any other
error, by falling back to anything weaker than a net-v2 session: no net-v1
item (item version 1), no plaintext link, no session without the X25519
exchange and no weaker suite. net-v2 has no negotiation, so there is nothing
weaker to choose. The Companion treats `version` as a failed attempt and tells
the person the Device doesn't speak net-v2, so its Firmware needs updating
over USB. It doesn't retry by itself and doesn't delete the Pairing. A Device
reply that is itself an item with version 1 is malformed and closes the carrier
connection. If a later protocol adds versions, it MUST choose among them with
something authenticated, never from an unauthenticated error.

Errors close the carrier connection,
except that the first two `auth` errors after a pairing proof return to code
entry. After a terminal error, TCP drains input for at most 100 ms before
closing, so the peer reads the error instead of a reset; BLE waits at most 100 ms after the
notification before disconnecting. An established session closes silently on
any cryptographic or framing error. Once `session.ready` is sent, no plaintext
error is valid.

## Pairing

Pairing is net-v1's, with item version 2 and labels under `touchdeck/net-v2/`.
It is the same on both Carriers, and one Pairing serves both. Field names
below follow the glossary; the wire bytes are the same as net-v1's except the
version byte and the label prefix.

The first item on a carrier connection is `pair.request` or `session.open`.

**Pairing windows open only on the Device.** Nothing on a network Carrier can
open one. A window opens when a person chooses "Pair new app" on the Device,
or from USB, which needs the same physical access: the console's `net pair`
command or the USB-only `net.pair` message below. The window lasts 120000 ms
from opening on a monotonic clock, and the Device shows it full screen with a
Cancel button and the time left. A `pair.request` with no open window gets
`closed`. A Companion tells the person to open the window on the Device and
try again; it never retries by itself.

The first `pair.request` while a window is open binds the window to that
carrier connection, which becomes the owner. The Device then shows a fresh
six-digit decimal code, zero-padded, with the requesting `companion_name`.
The code is sampled uniformly from 0 to 999999 by rejection sampling. It never
leaves the display or the SRP computation. `pair.offer` reports the time left
in the window.

`companion_id` is a random 16-byte id, stable for one Companion installation.
`companion_name` is 1 to 64 bytes of valid UTF-8; `name_len` counts bytes.
The name labels the Pairing for a person and grants no authority. It MUST NOT
contain a control character (`\p{Cc}`, which includes NUL) or a format
character (`\p{Cf}`). Format characters are invisible or reorder text, so
they can make one name look like another on screen: the bidirectional marks
LRM, RLM and ALM, the overrides and isolates U+202A to U+202E and U+2066 to
U+2069, the zero-width characters U+200B to U+200D and U+2060, the byte order
mark U+FEFF (also as the first character), the soft hyphen and the tag
characters. Rejecting U+200D also rejects emoji joined into one glyph; single
emoji are fine. So that a receiver needs no Unicode database, the `\p{Cf}` set
is pinned to Unicode 17.0's, these inclusive hex ranges, also in the vectors'
`companion_names.format_ranges`: `00ad`, `0600-0605`, `061c`, `06dd`, `070f`,
`0890-0891`, `08e2`, `180e`, `200b-200f`, `202a-202e`, `2060-2064`,
`2066-206f`, `feff`, `fff9-fffb`, `110bd`, `110cd`, `13430-1343f`,
`1bca0-1bca3`, `1d173-1d17a`, `e0001` and `e0020-e007f`. `\p{Cc}` is
U+0000 to U+001F and U+007F to U+009F. The name also MUST NOT contain the
Hangul fillers U+115F, U+1160, U+3164 and U+FFA0, letters (`\p{Lo}`) that
render as blank space. A refused name closes the carrier connection. A
link-v2 Device name obeys the same character rule, the C1 controls U+0080 to
U+009F included ([link-v2.md](link-v2.md#device-name)); both specs share the
names in [name-vectors-v2.json](name-vectors-v2.json), whose `control_ranges`
list `\p{Cc}`.
`window_id` is a fresh random 16-byte value. A new request can't replace or
extend an open window. Three failed client proofs close the window and the
carrier connection.

Cancel, carrier close, commit or expiry destroys the window, code, salt,
verifier, private exponents and temporary keys. A failed proof destroys that
attempt's SRP state but keeps the window and its failure count. Each retry
uses fresh exponents, salt and nonces. An invalid public value or malformed
item closes the carrier connection and the window.

### Failure backoff

Windows need a person at the Device, but a person can be tricked into opening
one, and a nearby attacker can race the real Companion to bind it. So the
Device also limits code guesses across windows and reboots:

* `pair_failures` is a counter in NVS. Each failed client proof increments it,
  and the Device commits it to NVS before it sends `auth`. A storage failure
  closes the window as if it were locked out.
* A dropped window counts too. When the carrier connection bound to a window
  ends before a commit for any reason other than Cancel on the Device, the
  window expiring, a storage failure or a failed proof already counted, the
  Device increments `pair_failures` once and commits it to NVS, and the
  lockout below applies. That covers a peer close, a radio loss, a missed
  deadline and a malformed item, so binding a fresh window and dropping it
  isn't a free retry. A radio loss counts here, unlike in per-peer backoff,
  because a dropped window costs the person at the Device a new window.
* When `pair_failures` reaches 3 or more after an increment, the Device closes
  the window and the carrier connection and locks pairing for
  `min(30000 ms * 2^(pair_failures - 3), 900000 ms)`. That is 30 s, 60 s,
  120 s and so on, capped at 15 minutes, so steady guessing gets at most 96
  tries a day.
* While locked, the Device opens no window. Its pairing screen shows the
  number of failed codes and the time left, and `pair.request` gets `closed`.
* The time left is stored in NVS whenever the lock starts, and at least once a
  minute while it runs. After a reboot the lock resumes from the stored value,
  so restarting the Device doesn't clear it.
* A committed Pairing resets `pair_failures` to 0 and clears the lock. So does
  wipe-all (`net.forget` with `all:true`), which is USB-only. Forgetting one
  Pairing leaves both alone.

| Kind | Name | Sender | Fields |
|---|---|---|---|
| `0x10` | `pair.request` | Companion | `companion_id[16], name_len:u8, companion_name[name_len]` |
| `0x11` | `pair.offer` | Device | `device_id[6], window_id[16], remaining_ms:u32be` |
| `0x12` | `pair.start` | Companion | `A[384], client_nonce[32]` |
| `0x13` | `pair.challenge` | Device | `salt[16], B[384], device_nonce[32]` |
| `0x14` | `pair.proof` | Companion | `client_proof[32]` |
| `0x15` | `pair.accept` | Device | `device_proof[32], wrapped_key[48]` |
| `0x16` | `pair.commit` | Companion | `commit_proof[32]` |
| `0x17` | `pair.stored` | Device | `stored_proof[32]` |

`E(kind, fields)` is a complete item, prefix included. Numbers in brackets are
byte lengths. The Device waits for code entry after `pair.offer`, and the
Companion sends `pair.start` only after the person enters the code. A pairing
attempt has a 10000 ms deadline from the first byte of `pair.start`, covering
all crypto, proofs, persistence and `pair.stored`. The first item must arrive
within 2000 ms of the carrier connection starting. Waiting for the person is
the only wait outside these deadlines. Partial reads never extend a deadline.

### SRP arithmetic

Unchanged from net-v1. `N` is the 3072-bit prime in RFC 5054 Appendix A and
`g = 5`. `PAD(n)` is the 384-byte big-endian encoding and `MIN(n)` the
shortest nonempty one. `INT(bytes)` reads an unsigned big-endian integer.

```text
I = "touchdeck:" || hex(device_id) || ":" || hex(companion_id)
P = the six ASCII code digits
a, b = independent random nonzero 32-byte unsigned integers
k = INT(H512(PAD(N) || PAD(g)))
x = INT(H512(salt || H512(I || ":" || P)))
v = g^x mod N
A = g^a mod N
B = (k*v + g^b) mod N
u = INT(H512(PAD(A) || PAD(B)))
S_companion = (B - k*g^x mod N)^(a + u*x) mod N
S_device = (A * v^u mod N)^b mod N
K_srp = H512(MIN(S))
```

The receiver MUST require `0 < A < N` or `0 < B < N`, and `u != 0`, and MUST
abort on `S = 0` or a zero client subtraction base. Public values are always
padded to 384 bytes. Salts keep all 16 bytes, including leading zeros. No
verifier or password hash is sent. net-v1's leading-zero vectors and its notes
on ESP-IDF `esp_srp` and RustCrypto `srp` padding apply unchanged.

### Transcript, proofs and key delivery

`request`, `offer`, `start` and `challenge` are the complete items. A retry
keeps the original request and offer and replaces start and challenge.

```text
T_pair = H("touchdeck/net-v2/pair\0" || request || offer || start || challenge)
PRK_pair = HKDF-Extract-SHA256(salt = T_pair, IKM = K_srp)
derive(PRK, label) = HKDF-Expand-SHA256(PRK, "touchdeck/net-v2/" || label, 32)
kc = derive(PRK_pair, "pair/client")
kd = derive(PRK_pair, "pair/deck")
kw = derive(PRK_pair, "pair/wrap")
ka = derive(PRK_pair, "pair/commit")
client_proof = MAC(kc, T_pair)
device_proof = MAC(kd, T_pair || client_proof)
commit_proof = MAC(ka, "commit" || T_pair || H(LTK))
stored_proof = MAC(ka, "stored" || T_pair || H(LTK))
```

The labels keep net-v1's words (`deck`, `client`) so the old code's
derivation carries over; only the prefix changes.

The Device verifies `client_proof` before it sends its proof or creates the
long-term key. It generates a fresh random 32-byte `LTK` and seals it with
AES-256-GCM under `kw`, a nonce of twelve zero bytes, and associated data
`T_pair || client_proof || device_proof`. `wrapped_key` is ciphertext then
tag. The zero nonce is used once with this attempt's unique key. The Companion
verifies `device_proof` before it opens the envelope.

The Companion writes a pending credential to the OS credential store before
`pair.commit`. After checking `commit_proof`, the Device atomically stores
`{companion_id, companion_name, LTK}` in NVS and sends `pair.stored`. The
Companion marks the Pairing confirmed only after checking `stored_proof`. The
Device then ends the carrier connection. A fresh carrier connection opens a
session with the LTK.

If `pair.stored` is lost, the Companion keeps the pending key and tries a
session. Success confirms it. Failure leaves it pending until the person
retries or forgets it. While USB is online the session gets `busy`, so the
Companion confirms later. An uncommitted key on the Device is discarded on
disconnect. The Device refuses an existing `companion_id` and never overwrites
or evicts a Pairing. It stores at most four, and refuses `pair.request` with
`full` before it shows a code.

### Pairing slots

The Device has four slots. A slot frees only when someone removes its Pairing:
a Pairing never expires, an unused one isn't evicted, and a new Pairing never
replaces an old one. There are three ways to remove one.

* **On the Device.** The paired apps screen lists each Pairing's
  `companion_name`. It offers Forget for one and Forget all for every one, and
  each asks for confirmation on the Device before it erases anything.
* **Over USB, one app.** `net.forget` with `app_id`, below.
* **Over USB, all.** `net.forget` with `all:true`, below. It needs no
  confirmation on the Device, because USB already needs physical access.

Removing Pairings is never a network operation. A network session can't list
or forget Pairings, as the message table says, so a stolen Pairing can't revoke
the others. Removing a Pairing erases its entry in NVS durably, drops its
update prompt cooldown, and closes that app's session if it has one. A session
that ends this way gets no error item, only a close. The app's next
`session.open` gets `auth`, because its `open_tag` matches no Pairing. The
Companion then tells the person that the Device no longer knows this app and
that they can pair again. It keeps its own key until the person forgets the
Device, because an `auth` is an unauthenticated hint and never deletes a key.

**A full Device tells the person how to free a slot.** With four Pairings:

* "Pair new app" on the Device opens no window. It shows "Four apps are paired.
  Forget one first." with the paired apps list and its Forget buttons.
* `net.pair` over USB fails with `reason:"full"` and the same sentence as its
  `error`: `Four apps are paired. Forget one first.` It fits the 96-byte limit.
* A `pair.request` that gets `full` or `closed` makes the Companion look at the
  advertised `paired` count. A count of 4 means it tells the person: "This
  Device has four paired apps. Remove one on the Device (paired apps screen),
  or over USB, then pair again." The Companion shows the same advice up front
  when the TXT record or the BLE service data already says 4, before the
  person goes to the Device. `paired` is a hint; the Device's own answer is
  the one to trust.
* The Companion's USB view lists the Pairings with `net.list` and offers
  Forget on each, so the person can free a slot without touching the Device.

## Session setup

Every session, including the first after pairing, runs this exchange. Its
deadline is 10000 ms from the carrier connection starting. Each side makes a
fresh X25519 key pair for this session only: `e_c` and `E_c = X25519(e_c, 9)`
for the Companion, `e_d` and `E_d` for the Device.

| Kind | Name | Sender | Fields |
|---|---|---|---|
| `0x20` | `session.open` | Companion | `open_tag[16], client_share[32]` |
| `0x21` | `session.challenge` | Device | `device_share[32], device_proof[32]` |
| `0x22` | `session.proof` | Companion | `client_proof[32]` |
| `0x23` | `session.ready` | Device | `ready_proof[32]` |

`client_share` is `E_c` and `device_share` is `E_d`, each the 32-byte
little-endian u-coordinate from RFC 7748. The fresh shares replace net-v1's
session nonces.

`session.open` names neither the Device nor the Companion. Instead it carries
`open_tag`, which only the holder of a Pairing's LTK can make, for this one
`client_share`:

```text
K_open = derive(LTK, "session/open")
open_tag = the first 16 bytes of MAC(K_open, "open" || client_share)
```

```text
T_session = H("touchdeck/net-v2/session\0" || device_id || companion_id || client_share || device_share)
Z = X25519(e_c, device_share) = X25519(e_d, client_share)
PRK_session = HKDF-Extract-SHA256(salt = T_session, IKM = LTK || Z)
kd = derive(PRK_session, "session/deck")
kc = derive(PRK_session, "session/client")
kr = derive(PRK_session, "session/ready")
key_c2d = derive(PRK_session, "session/c2d")
key_d2c = derive(PRK_session, "session/d2c")
device_proof = MAC(kd, T_session)
client_proof = MAC(kc, T_session || device_proof)
ready_proof = MAC(kr, T_session || client_proof)
```

The Device finds the Pairing by computing `open_tag` for all four slots,
using a fixed dummy key for empty slots, and comparing each with the received
tag in constant time. Empty slots never match. It computes all four tags
before it decides, so the work reveals neither the Pairing count nor which
one matched. The matching Pairing gives `companion_id` and the LTK, and its own `device_id`
completes `T_session`. A tag that matches no Pairing gets `auth` before any
X25519 work. A Companion sends `session.open` only to the Device of the
Pairing whose key made the tag, and the selected `hello`'s `id` check below
still pins the Device.

**Why no ids on the wire.** net-v1 and earlier drafts of net-v2 sent
`device_id` and `companion_id` in cleartext in every `session.open`. Anyone on
the path could then follow a Companion from network to network and see which
Device it talks to. With `open_tag`, a passive observer sees 16 bytes that
change with every session. Finding the Pairing costs the Device four HMACs,
because it holds at most four. `companion_id` travels in cleartext only in
`pair.request`, while a person pairs on purpose, and `device_id` only in
`pair.offer` and in pairing mode's TXT record.

**Share checks.** Every implementation follows this one rule for the peer's
share, so a bad share fails the same way whatever its X25519 library does:

1. The share MUST be canonical: bit 255 clear and `u < p`, where
   `p = 2^255 - 19`. `X25519(e, 9)` always produces such a share. The
   receiver checks this itself before any library call, because RFC 7748
   masks bit 255 while libraries differ on values at or above `p`.
2. Then the receiver computes `Z`. It treats a share its library refuses on
   import and an all-zero `Z` (RFC 7748 section 6.1) as the same failure.
   Mbed TLS PSA refuses some small-order points on import; x25519-dalek
   accepts them and returns an all-zero `Z`, which
   `SharedSecret::was_contributory()` reports. Either way the session fails.

A failed share check closes the carrier connection with no error item and
counts as a malformed item for per-peer backoff. The Device sends no
`session.challenge`, and the Companion sends no `session.proof`. Pairings are
unchanged. The vectors' `negative.x25519` lists all twelve small-order
encodings and an honest share with bit 255 set; each MUST close.

The Companion checks `device_proof` before it
sends its proof. The Device checks `client_proof` before `session.ready`. The
Companion checks `ready_proof` before it sends a record. A failed check closes
the carrier connection and erases the ephemeral keys, without deleting the
Pairing. Each side erases its X25519 private key once it has `Z`, and erases
`Z` and `PRK_session` once the five keys are derived. A session shows in
`net.state` only after the Device sends `session.ready`.

What this gives:

* Both proofs need the LTK, so only a paired Companion and its Device can
  finish the exchange. The shares are inside the transcript, so an attacker
  can't swap them.
* Traffic keys depend on `Z`. Someone who records a session and later steals
  the LTK, from NVS or the OS credential store, still can't compute `Z` and
  can't decrypt it. That is forward secrecy, which net-v1 lacked. A stolen LTK
  still lets the thief open new sessions until the Pairing is removed.
* Pairing resists offline code guessing from a passive capture or a fake
  Device that lacks the code. An active impersonator can test one guess per
  exchange, three per window on the real Device, and every window needs a
  person at the Device. The failure backoff caps guesses at 96 a day across
  windows and reboots. The Companion MUST ask the person again after a failed
  proof and MUST NOT retry a code by itself.
* A Pairing still isn't USB. A network session can't change Wi-Fi
  credentials, open pairing windows or wipe Pairings, and can install Firmware
  only after the person at the Device approves it.

## Records

Each direction has its own counter, starting at zero. Direction 0 is Companion
to Device, 1 is Device to Companion. Counters are implicit; only the next
expected one is accepted.

```text
nonce = 00000000 || u64be(counter)
len = plaintext.length + 16
AAD = "TDN2" || direction:u8 || u64be(counter) || u16be(len)
body = AES-256-GCM(key_direction, nonce, plaintext, AAD)
item = u16be(len) || body
```

The sender serializes writes per direction, increments its counter once per
sealed record, and never retries a partly written record on the same carrier
connection. A link chunk retransmission is a new record with a new counter.
The receiver authenticates before it exposes plaintext or refreshes liveness,
then checks the whole link frame body: CRC, frame version, header and exact
payload length. Any AEAD failure, replay, skipped or reordered record, invalid
body, end of stream mid-item, or impossible length closes the session. A valid
frame with an invalid message follows link-v1 dispatch. Nobody scans for a new
record after a framing error.

**Record limit: 2^24 per key.** Each direction carries at most `2^24` records
(counters 0 to `2^24 - 1`) under its key. That is about `2^30` AES blocks,
in line with TLS 1.3's guidance for AES-GCM and well below net-v1's `2^32`
records. Counters MUST NOT wrap. A sender doesn't seal counter `2^24`, and a
receiver closes the session when it would have to accept it, even with a valid
tag. **The session ends at the limit and doesn't rekey.** Rekeying would need
an authenticated in-session message, a second code path and a transition that
two implementations could disagree on. Ending costs one reconnect: the
Companion opens a new carrier connection and runs the session exchange again,
which makes fresh X25519 shares and keys with counters at zero. Counters and
keys never outlive the carrier connection. A `ping` every 2000 ms alone takes
about 388 days to reach the limit, so ordinary use meets it only in a long
transfer or a very long-lived session. A sender SHOULD end the session between
requests, not in the middle of one, once fewer than 1024 records remain in
either direction. A request the limit interrupts is unanswered, so the next
section applies to it.

## Close and unknown outcomes

net-v2 has no authenticated close. A TCP FIN or a BLE disconnect that falls on
a record boundary looks the same whether the other side meant it, the radio
dropped it, or someone on the path cut the stream there. A cut inside a record
is caught, because the record's length or tag fails. So:

* A request is **answered** only when its response record has arrived and
  authenticated. When a session ends for any reason, every request without an
  answer has an **unknown outcome**: the Device may not have received it, may
  have received it and not acted, or may have acted and lost the reply.
* The Companion MUST NOT treat an unanswered request as succeeded or as
  failed. It shows "unknown" until it learns the outcome as below.
* A close is never proof of anything. In particular it isn't proof that the
  Device wiped, stored or removed something.

The Companion recovers after reconnecting, by reading the state back or by
repeating a request that is safe to repeat. The Device never replays a reply
into a new session.

| Unanswered request | Recovery |
|---|---|
| `net.forget` with `app_id` or `all:true` (USB-only, and the rule holds for a lost USB reply too) | Repeat it. Forgetting an id that is gone succeeds with `removed:0`, and `all:true` on an empty list succeeds too. Or read `net.list`. The Companion keeps its local key, marked as being removed, until it sees the Pairing gone. The person can still discard the key locally. |
| `layout.put` and `xfer.end`, `icon.put` and `xfer.end` | The Device writes only at `xfer.end`, so a cut transfer leaves nothing. If the `xfer.end` reply was lost, the file may be stored. Read it back with `layout.get` (size and CRC-32) or `icon.list`, and repeat the whole put if it differs. |
| `v2.asset.put` and `v2.asset.end` | Repeat `v2.asset.put`. `present:true` means the Asset is committed; otherwise start the transfer again from chunk 0. |
| `v2.photo.put` and `v2.photo.end` | Repeat the photo from the start. Showing the same photo twice is harmless. |
| `update.begin`, its chunks and `update.end` | The Device restarts after `update.end`, so a lost reply is expected. Reconnect and read `hello`: if `version` and `elf_sha256` are the new image's, it installed. Otherwise the old image still runs. The person decides whether to try again, because each network attempt needs the Device's approval again. |
| `icon.rm`, `v2.asset.delete` | Repeat. `no such icon`, or `deleted:false`, means it is already gone. |
| `settings.put`, `v2.name.set`, `v2.idle.set` | Read back with the matching `get` and repeat if it differs. |
| `v2.clock.set`, `widget.set`, `dashboard.set`, `v2.live.set` | The latest value wins, so send the current one again. `v2.live.set` takes a new `sequence`. |
| `hello`, `ping`, `link.state`, `net.state`, `layout.get` and other reads | Repeat. Reads change nothing. |

A transfer the Device had open when the carrier connection closed is aborted
and its temporary data discarded, as link-v2 says. A new session starts every
transfer from the beginning.

## Link protocol selection and liveness

After `session.ready`, the Companion runs link-v2's negotiation exactly as on
USB. It sends the plain `{"m":"hello"}`. A Device that implements net-v2 MUST
answer with `protocols` containing 2. The Companion then sends
`{"m":"hello","proto":2}`, and the reply selects protocol 2 for this session
and carries `id`, `name` and `caps`. All link-v2 rules apply: re-selection,
transfer aborts, live value resets, error reasons and limits. The one
addition is which messages may run, in the next section.
The Companion's link session is the same code on every Carrier; only frame
delivery differs.

The Companion sends `hello` at once after `session.ready` and `ping` every
2000 ms, even with other traffic. The link is online on `hello` and expires
5000 ms after the last valid authenticated frame. On a network Carrier, expiry
closes the carrier connection rather than only resetting selection. Before the
first `hello`, the Device closes 5000 ms after `session.ready`. The Companion
closes after 5000 ms without a valid frame from the Device. Partial items and
invalid bytes refresh nothing. Closing the session releases the Companion's
held keys and aborts its transfers.

## Messages on a network Carrier

A Pairing can be guessed at or its key stolen, so a network session gets less
power than USB, which needs physical access. The Device MUST enforce this list
itself, under either selected link protocol. It applies to requests the
Companion sends; Device events such as `press`, `key.report`, `v2.input` and
`v2.page` flow as on USB.

| Rule | Messages |
|---|---|
| Allowed | `hello`, `ping`, `link.state`, `layout.get`, `layout.put`, `icon.put`, `icon.list`, `icon.rm`, `settings.get`, `settings.put`, `store.stat`, `xfer.end`, `widget.set`, `dashboard.set`, `v2.name.get`, `v2.name.set`, `v2.asset.stat`, `v2.asset.list`, `v2.asset.put`, `v2.asset.end`, `v2.asset.delete`, `v2.live.set`, `v2.photo.put`, `v2.photo.end`, `v2.idle.get`, `v2.idle.set`, `v2.clock.set`, `net.state` |
| Confirmed on the Device | `update.begin`, then the update's chunks and `update.end` as usual |
| USB only | `net.pair`, `net.list`, `net.forget`, every `wifi.*` message, every `ble.*` message (link-v2.md, Bluetooth keyboard), and every message not named above |

Chunks and acks are allowed only for a transfer this session opened. A
USB-only request gets `ok:false`, `reason:"usb-only"` and
`error:"Only over USB"`, and changes nothing. A contract that adds a message
MUST say which row it belongs to; until it does, the message is USB-only.

**Firmware update over a network Carrier.** The person at the Device approves
each image:

1. On a network session, `update.begin` doesn't open a transfer. The Device
   shows a full-screen prompt for 60000 ms: the requesting `companion_name`,
   the image size, the first 8 hex digits of `sha256`, and Install and Cancel
   buttons. It replies `ok:false`, `reason:"confirm"`.
2. The Companion repeats the same `update.begin`, with the same `bytes` and
   `sha256`, at most every 2000 ms, and tells the person to look at the
   Device. Repeats get `confirm` while the prompt is up.
3. After the person taps Install, the next matching `update.begin` from the
   same session within the 60000 ms opens the transfer, with link-v1's normal
   reply. The rest of the update, its checks and its rollback are link-v1's.
4. Cancel, the prompt timing out, the session closing, or a repeat with a
   different `bytes` or `sha256` ends the prompt unapproved. The next
   matching repeat gets `reason:"declined"`.
5. **Prompt cooldown.** A prompt that ends unapproved starts a cooldown for
   its Pairing when it ends: 60000 ms for the first, doubling for each
   further unapproved prompt up to 3600000 ms. During the cooldown every
   `update.begin` from that Pairing, on any session, gets
   `reason:"declined"` at once and shows nothing. The first `update.begin` after the cooldown ends starts a new
   prompt. Tapping Install resets the cooldown. The Device keeps it in RAM per
   Pairing, so a paired peer can hold the screen for at most one 60000 ms
   prompt per hour once the cap is reached, and reconnecting doesn't help it.

A Companion that gets `declined` tells the person the update was declined on
the Device and doesn't repeat `update.begin` by itself; the person can try
again later or update over USB.

`confirm` and `declined` are added to link-v1's update reasons for network
sessions only. Over USB, `update.begin` works as before.

## Timeouts and limits

| Item | Value |
|---|---|
| BLE LE connection to both notifications enabled | 5000 ms |
| First handshake item after the carrier connection starts | 2000 ms |
| Session setup, from the carrier connection starting | 10000 ms |
| Pairing window, from opening on the Device | 120000 ms |
| Pairing attempt, from the first byte of `pair.start` | 10000 ms |
| First `hello` after `session.ready` | 5000 ms |
| `ping` interval, link expiry | 2000 ms, 5000 ms |
| `busy` write, terminal drain | 100 ms each |
| Handshake body | 512 bytes |
| Record body (`len`) | 24 to 1048 bytes |
| Item on the wire | at most 1050 bytes |
| BLE fragment sent | 1 to `ATT_MTU - 3` bytes, at most 20 when the MTU is unknown |
| BLE fragment received | 1 to `ATT_MTU - 3` bytes, at most 514 when the MTU is unknown |
| BLE initial credit | at least 1050 bytes, 2100 recommended |
| Records per direction per key, then the session ends | `2^24` |
| Stored Pairings | 4 |
| Failed proofs per pairing window | 3 |
| Pairing lockout after the third and later failures | 30000 ms doubling, at most 900000 ms |
| Per-peer backoff, peer-caused failures only, skipped by a stored Pairing's `open_tag` | 1000 ms doubling, at most 60000 ms |
| Firmware update prompt on the Device | 60000 ms |
| Update prompt cooldown per Pairing after an unapproved prompt | 60000 ms doubling, at most 3600000 ms |

## The long-term key is symmetric

Both ends hold the same `LTK`, and each side's proofs are MACs under keys
derived from it. Nothing in net-v2 tells the Device and the Companion apart by
secret, so whoever has a copy of the key can play either side:

* A copy from the Companion's OS credential store lets the thief open sessions
  to the Device as that Companion, which the Pairing already allows for any
  holder of the key.
* A copy from the Device's NVS does that too, and also lets the thief
  impersonate the Device to the Companion. The Companion accepts any peer that
  proves it holds the LTK for that `device_id`, and a fake Device can then
  answer link requests with chosen state and read everything the Companion
  sends it.
* Forward secrecy doesn't change this. It keeps recorded sessions closed, not
  new ones.

The Device's NVS isn't encrypted in the radio build, so a flash dump over USB
(or a desoldered chip) yields every stored Pairing. NVS encryption needs an
eFuse burn, so it waits for an operator decision
([ADR 0010](../../docs/adr/0010-nvs-stays-unencrypted-until-the-operator-decides.md)).
Until then:

* Treat physical access to the Device as access to every Pairing on it.
* After a Device is lost, resold or serviced by someone else, run `net.forget`
  on it (or reflash and erase NVS), and forget the Device in each Companion.
  Removing the Pairing from only one side doesn't end the other side's key.
* Companions SHOULD tell the person this when they pair, and MUST NOT describe
  a Pairing as proving the Device's identity against someone with its hardware.

A design with a per-side secret (for example a Device identity key that the
Companion pins, with the LTK only for the session) would close the
impersonation half. It is a protocol change and out of scope for net-v2.

## Storage, management and secrets

The Device stores Pairings in NVS, separate from Layouts and Wi-Fi
credentials, and unencrypted (see above). net-v1 never shipped, so there's nothing to migrate. The
Companion keeps LTKs in the OS credential store (Windows Credential Manager or
the Secret Service on Linux, for example through the `keyring` crate). It may
keep `device_id`, `companion_id` and the Device name in ordinary settings.
Forgetting a Device in the Companion deletes its local key and closes its
session; it doesn't revoke the Device's entry. Removing the Pairing on the
Device, or `net.forget` over USB, does.

These link messages use the normal link response envelope. Their JSON names
stay as in net-v1 ([ADR 0003](../../docs/adr/0003-brand-changes-wire-names-stay.md)).
`net.state` works over USB and over a net-v2 session. `net.pair`, `net.list`
and `net.forget` are USB-only.

| Request | Successful response fields |
|---|---|
| `{"m":"net.state"}` | `listening` Boolean, `port:47820`, `ble` Boolean, `session` null or `{app_id,app_name,carrier,since_ms}`, `paired` 0 to 4, `pairing` Boolean, `pair_failures` integer, `locked_ms` integer |
| `{"m":"net.pair"}` | The same fields after opening a pairing window |
| `{"m":"net.list"}` | The same fields as `net.state`, plus `free` (0 to 4) and `pairings`, an array of `{app_id,app_name}` for every stored Pairing, oldest first |
| `{"m":"net.forget","app_id":"<32 hex>"}` | The same fields after removing that one Pairing, plus `removed`, 1 or 0 |
| `{"m":"net.forget","all":true}` | The same fields after clearing every Pairing, plus `removed`, the number cleared |

`app_id` is `companion_id` as 32 lowercase hex characters and `app_name` is
`companion_name`. `carrier` is `"wifi"` or `"ble"`. `since_ms` is the
Device's monotonic time at `session.ready`. `listening` reflects the TCP
listener and `ble` whether the link service is advertising or connected.
`pairing` is true while a window is open. `pair_failures` is the stored
counter and `locked_ms` the time left in a pairing lockout, 0 when none.
`net.pair` fails with `reason:"locked"` during a lockout and `reason:"full"`
when four Pairings exist. A failure is otherwise `ok:false` with an `error`
sentence and `reason:"storage"`. `net.forget` with `app_id`
removes one Pairing and nothing else. It is idempotent: an `app_id` that isn't
stored succeeds with `removed:0`, so a Companion that lost the reply can send it
again. An `app_id` that isn't 32 lowercase hex characters, or a request with
both `app_id` and `all`, fails with `reason:"args"`. It ends that app's session
if it has one. `net.forget` with `all:true` cancels a window, erases every
Pairing durably, resets `pair_failures` and the lockout, and ends any network
session. A bare `net.forget` with neither field fails with `reason:"args"`, so
a request that lost its `app_id` can't wipe the Device. A failed erase still
ends the affected session and reports the failure, and keeps the Pairings it
couldn't erase, so a list or a retry sees what storage still holds. Over a network Carrier all
three messages get `usb-only`.

Codes, verifiers, private exponents, X25519 private keys, `Z`, LTKs and
session keys MUST NOT appear in logs, console replies, `hello`, screenshots
other than the pairing display, crash reports or evidence. Synthetic fixture
values are the only exception.

## Resource limits

net-v1's budget applies to whichever network Carrier is active: 16 KiB steady
state for the network owner, up to 64 KiB during SRP, and at most 8 KiB of
either in internal DRAM. X25519 adds a few hundred bytes per session. There is
one SRP context, one 512-byte handshake buffer, one 1048-byte receive record
body, one 1048-byte transmit record body, and on BLE one reassembly buffer
sized to the credit the Device grants. lwIP, Wi-Fi and NimBLE buffers are
outside this budget and still count against the radio build's 24 KiB
internal-RAM floor. Crypto runs below UI priority, never in a UI, USB or BLE
host callback.

## Implementation notes

These were checked on 2026-11-06. They point at the primitives the earlier
`td_netlink_core` (Firmware) and `td-link` `net.rs` (Companion) code already
used: SRP-6a 3072/SHA-512, AES-256-GCM, HMAC and HKDF-SHA256 through Mbed TLS
PSA on the Device, and `srp`, `aes-gcm`, `hkdf`, `hmac` and `sha2` in Rust.

* **Device X25519.** The ESP-IDF 6.1 tree has
  `CONFIG_MBEDTLS_ECP_DP_CURVE25519_ENABLED`, default `y`, which its
  `esp_config.h` maps to `PSA_WANT_ECC_MONTGOMERY_255`. Use `psa_generate_key`
  with `PSA_KEY_TYPE_ECC_KEY_PAIR(PSA_ECC_FAMILY_MONTGOMERY)`, 255 bits, and
  `psa_raw_key_agreement(PSA_ALG_ECDH, ...)`. The radio build's sdkconfig must
  keep the option on.
* **Companion X25519.** `x25519-dalek` 2 with `EphemeralSecret` and
  `SharedSecret::was_contributory()` for the all-zero check.
* **Companion BLE.** One GATT client that runs on WinRT and BlueZ, for example
  `btleplug`. It writes `rx` without response and subscribes to `tx` and
  `flow`. It reads the ATT MTU only where the platform reports it.
* **SRP.** net-v1's notes on `esp_srp` (log-tag cap, salt generation, no
  double hashing of `K_srp`) and on RustCrypto `srp` 0.6 (unpadded `u`) apply
  as written.

## Reference and conformance

The Node reference is [net-ref/net-v2.mjs](net-ref/net-v2.mjs). It reuses the
net-v1 reference's SRP arithmetic and link frame checks, and uses only Node's
built-in crypto. [net-ref/vectors-v2.mjs](net-ref/vectors-v2.mjs) regenerates
the vectors from fixed public inputs; the test checks the committed file
without rewriting it. Run from the repository root with Node 22 or later:

```powershell
node --test schema/protocol/net-ref/net-v2.test.mjs schema/protocol/net-ref/net-v2-negative.test.mjs
node schema/protocol/net-ref/vectors-v2.mjs --write
```

[net-v2-vectors.json](net-v2-vectors.json) holds:

* `pake` and `pairing`: SRP values, every pairing item, the transcript, keys,
  proofs and the wrapped LTK.
* `session`: both X25519 shares, `Z`, `K_open` and `open_tag`, the
  transcript, keys, proofs and the four session items.
* `discovery`: `K_disc`, the nonce, the three fillers, the fixture Pairing's
  hint, the label, instance and host names, and the TXT record out of and in
  pairing mode. `other_record` is a record with another nonce and only
  fillers, which matches no Pairing.
* `ble_discovery`: the compact service data for an occupied slot, three fillers,
  rotation, an unpaired Device, pairing mode and a full four-Pairing store.
* `records`: the discovery and selected link-v2 hellos, two records each way,
  built from [golden-frames-v2.json](golden-frames-v2.json). The selected
  reply carries the fixture `device_id` as its `id`. The two selected records
  include their BLE fragments at ATT_MTU 23 and 247.
* `tampered_record`, which MUST close the receiver, and
  `non_contributory_share`, an all-zero share that MUST close the handshake.
* `companion_names`: the pinned `\p{Cf}` ranges, `pair.request` items whose
  names every receiver MUST accept, and items with one control or format
  character or Hangul filler, labelled by code point, that MUST close the
  carrier connection.
* `errors.busy`, `errors.version` and `errors.closed`, and the `ble` UUIDs and
  recommended initial credit.
* `management`: ten ordered `net.list`, `net.pair` and `net.forget` requests on a
  store of four synthetic Pairings, each with its carrier and the exact
  response: a full Device, forgetting one app, repeating it, `usb-only` over
  Wi-Fi and BLE, the `args` refusals and wipe-all.
* `negative`: inputs every implementation MUST refuse, each with its
  receiver and expected outcome (`auth`, `close`, or `accept` for the
  boundary case):
  * `wrong_code`: a `pair.proof` from the wrong code after the fixture's
    pairing items. The Device sends `auth`.
  * `tampered_wrapped_key`: `pair.accept` with a flipped ciphertext or tag
    byte. The Companion closes without an LTK.
  * `swapped_client_share`: a `session.open` whose share was replaced under
    the original tag. The tag check fails and the Device sends `auth`.
  * `swapped_device_share`: a share replaced under an unchanged proof. The
    proof check fails and the Companion closes.
  * `unknown_open_tag` and `tampered_open_tag`: a `session.open` tagged with
    an LTK the Device doesn't hold, and the fixture's with one tag bit
    flipped. Both get `auth`.
  * `counter_limit`: a record at counter `2^24 - 1`, which opens, and one at
    `2^24` with a valid tag, which closes. The session ends there, and a new
    session starts at counter zero.
  * `version_error`: the `version` error item, which the Companion handles by
    stopping with no fallback, and the same item with item version 1, which
    closes.
  * `max_record`: a 1050-byte item, which opens, and `oversize_items`: length
    prefixes one past each phase's bounds, which close before allocation.
  * `write_before_credit` and `credit_overrun`: `rx` events, as enabled
    notifications and write lengths, that end the carrier connection.
  * `x25519`: the twelve small-order encodings and an honest share with bit
    255 set, which all close under the share checks.

Implementations also test reflection, replay, reorder, truncation, fragment
boundaries and middle-fragment sizes at ATT_MTU 23, the pairing lockout across
a reboot, which outcomes count toward per-peer backoff, the update prompt
cooldown, the network message list, `net.forget` and the slot rules, and the
unknown outcome of each request in the recovery table.
