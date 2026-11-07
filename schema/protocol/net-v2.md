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
  and wiping Pairings stay USB-only.

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
that block multicast need the typed address. The TXT record has these keys:

| Key | Value |
|---|---|
| `id` | Device id, 12 lowercase hex digits |
| `proto` | `2` |
| `name` | The Device name from link-v2, at most 48 UTF-8 bytes |
| `paired` | Number of stored Pairings, decimal |

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
same UUID: `proto:u8` (2), `device_id[6]`, `paired:u8`. The Companion matches
Devices by `device_id`, never by BLE address, because a Device MAY use a random
address. Both Windows (WinRT) and Linux (BlueZ, for example through btleplug)
report service data per UUID.

The link service needs no BLE pairing, bonding or link-layer encryption, and
the Companion MUST NOT start BLE pairing for it. net-v2 authenticates and
encrypts everything above GATT. Keeping the OS out of the trust decision means
no OS-level bond that reconnects on its own at boot, and the same Pairing works
on Wi-Fi and BLE. Bonds that the Device's Bluetooth keyboard role makes with
its hosts are separate and outside this document.

Advertised data and TXT records are untrusted hints. The Companion pins
`device_id` when it pairs and checks it in every session. An address change
doesn't change the Pairing.

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
  naming a stored `companion_id` takes over from an unproven owner that is not
  bound to a pairing window. The Device closes the old connection without an
  error and continues with the new one. A `session.open` with an unknown id
  gets `auth` and takes nothing over. An owner bound to a pairing window is
  never taken over; the person at the Device can cancel it.

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
  * `auth`, which includes an unknown `companion_id` and the failed pairing
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
  peer's first handshake item, as it does a takeover: a `session.open` that
  names a stored `companion_id` skips the backoff and goes on to the
  handshake, and any other first item gets `busy`. So someone who sends from
  the real Companion's public BLE address with an unknown id, which gets
  `auth` and counts against that address, can't keep the real Companion
  out. The Device keeps at least eight peers, evicting the one whose last
  failure is oldest. Because takeovers and refusals don't count, someone who
  copies the cleartext `companion_id` from a `session.open` on BLE can delay
  the real Companion's handshake but can't push its address into backoff.
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
| 2 | `version` | Unsupported item version |
| 3 | `args` | Invalid handshake fields or state |
| 4 | `full` | Four Pairings exist, or this Companion id already exists |
| 5 | `auth` | Authentication failed, including an unknown Companion id |
| 6 | `expired` | A deadline or the pairing window expired |
| 7 | `cancelled` | The person cancelled pairing on the Device |
| 8 | `storage` | The Pairing could not be committed |
| 9 | `closed` | No pairing window is open on the Device, or pairing is locked out |

For example, `busy` is hex `00037f0201`. Errors close the carrier connection,
except that the first two `auth` errors after a pairing proof return to code
entry. After a terminal error, TCP shuts down its write side and drains input
for at most 100 ms before closing; BLE waits at most 100 ms after the
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
render as blank space. A refused name closes the carrier connection.
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
  `net.forget`, which is USB-only.

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

## Session setup

Every session, including the first after pairing, runs this exchange. Its
deadline is 10000 ms from the carrier connection starting. Each side makes a
fresh X25519 key pair for this session only: `e_c` and `E_c = X25519(e_c, 9)`
for the Companion, `e_d` and `E_d` for the Device.

| Kind | Name | Sender | Fields |
|---|---|---|---|
| `0x20` | `session.open` | Companion | `device_id[6], companion_id[16], client_share[32]` |
| `0x21` | `session.challenge` | Device | `device_share[32], device_proof[32]` |
| `0x22` | `session.proof` | Companion | `client_proof[32]` |
| `0x23` | `session.ready` | Device | `ready_proof[32]` |

`client_share` is `E_c` and `device_share` is `E_d`, each the 32-byte
little-endian u-coordinate from RFC 7748. The fresh shares replace net-v1's
session nonces.

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

The Device checks `device_id` against its own and looks up `companion_id`. A
different `device_id` or an unknown `companion_id` gets `auth` before any
X25519 work.

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

Counters MUST NOT wrap. Close before sending or receiving counter `2^32`.
Reconnect to continue. Counters and keys never outlive the carrier connection.

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
| USB only | `net.pair`, `net.forget`, every `wifi.*` message, every `ble.*` message (link-v2.md, Bluetooth keyboard), and every message not named above |

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
| Records per direction per session | `2^32` |
| Stored Pairings | 4 |
| Failed proofs per pairing window | 3 |
| Pairing lockout after the third and later failures | 30000 ms doubling, at most 900000 ms |
| Per-peer backoff, peer-caused failures only, skipped by a stored `companion_id` | 1000 ms doubling, at most 60000 ms |
| Firmware update prompt on the Device | 60000 ms |
| Update prompt cooldown per Pairing after an unapproved prompt | 60000 ms doubling, at most 3600000 ms |

## Storage, management and secrets

The Device stores Pairings in NVS, separate from Layouts and Wi-Fi
credentials. net-v1 never shipped, so there's nothing to migrate. The
Companion keeps LTKs in the OS credential store (Windows Credential Manager or
the Secret Service on Linux, for example through the `keyring` crate). It may
keep `device_id`, `companion_id` and the Device name in ordinary settings.
Forgetting a Device in the Companion deletes its local key and closes its
session; it doesn't revoke the Device's entry. `net.forget` on the Device does.

These link messages use the normal link response envelope. Their JSON names
stay as in net-v1 ([ADR 0003](../../docs/adr/0003-brand-changes-wire-names-stay.md)).
`net.state` works over USB and over a net-v2 session. `net.pair` and
`net.forget` are USB-only.

| Request | Successful response fields |
|---|---|
| `{"m":"net.state"}` | `listening` Boolean, `port:47820`, `ble` Boolean, `session` null or `{app_id,app_name,carrier,since_ms}`, `paired` 0 to 4, `pairing` Boolean, `pair_failures` integer, `locked_ms` integer |
| `{"m":"net.pair"}` | The same fields after opening a pairing window |
| `{"m":"net.forget"}` | The same fields after clearing every Pairing |

`app_id` is `companion_id` as 32 lowercase hex characters and `app_name` is
`companion_name`. `carrier` is `"wifi"` or `"ble"`. `since_ms` is the
Device's monotonic time at `session.ready`. `listening` reflects the TCP
listener and `ble` whether the link service is advertising or connected.
`pairing` is true while a window is open. `pair_failures` is the stored
counter and `locked_ms` the time left in a pairing lockout, 0 when none.
`net.pair` fails with `reason:"locked"` during a lockout and `reason:"full"`
when four Pairings exist. A failure is otherwise `ok:false` with an `error`
sentence and `reason:"storage"`. `net.forget` cancels a window, erases every
Pairing durably, resets `pair_failures` and the lockout, and ends any network
session. A failed erase still ends the session and reports the failure.

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
* `session`: both X25519 shares, `Z`, the transcript, keys, proofs and the
  four session items.
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
* `errors.busy` and `errors.closed`, and the `ble` UUIDs and recommended
  initial credit.
* `negative`: inputs every implementation MUST refuse, each with its
  receiver and expected outcome (`auth`, `close`, or `accept` for the
  boundary case):
  * `wrong_code`: a `pair.proof` from the wrong code after the fixture's
    pairing items. The Device sends `auth`.
  * `tampered_wrapped_key`: `pair.accept` with a flipped ciphertext or tag
    byte. The Companion closes without an LTK.
  * `swapped_client_share` and `swapped_device_share`: a share replaced under
    an unchanged proof. The proof check fails and the receiver closes.
  * `wrong_device_id` and `unknown_companion_id`: `session.open` that gets
    `auth`.
  * `counter_limit`: a record at counter `2^32 - 1`, which opens, and one at
    `2^32` with a valid tag, which closes.
  * `max_record`: a 1050-byte item, which opens, and `oversize_items`: length
    prefixes one past each phase's bounds, which close before allocation.
  * `write_before_credit` and `credit_overrun`: `rx` events, as enabled
    notifications and write lengths, that end the carrier connection.
  * `x25519`: the twelve small-order encodings and an honest share with bit
    255 set, which all close under the share checks.

Implementations also test reflection, replay, reorder, truncation, fragment
boundaries and middle-fragment sizes at ATT_MTU 23, the pairing lockout across
a reboot, which outcomes count toward per-peer backoff, the update prompt
cooldown, and the network message list.
