# Network protocol reference

This reference implements the byte and crypto contracts in
[net-v1.md](../net-v1.md) and [net-v2.md](../net-v2.md). It needs Node 22 or later and no packages.
Its BigInt SRP code is a test oracle. Production code uses the documented
ESP-IDF and RustCrypto backends. JavaScript cannot guarantee secret erasure
or constant-time big-integer arithmetic.

Run the conformance tests from the repository root.

```powershell
node --test schema/protocol/net-ref/net.test.mjs
```

The test compares generated results with the committed JSON without rewriting
it. It also checks the existing link-v1 golden bodies, malicious public values,
wrong codes, transcript changes, authentication failures, record limits,
replay, reorder, truncation, and pairing deadlines.

`protocol.mjs` contains the SRP equations, typed handshake encodings, key
derivation, key wrapping, complete link body validation, directional record
states, and pairing-window state. `vectors.mjs` defines public deterministic
inputs and regenerates [net-v1-vectors.json](../net-v1-vectors.json).

```powershell
node schema/protocol/net-ref/vectors.mjs --write
```

Every secret in that JSON is a public fixture. Never use those values to pair
with a real deck. `S` uses its minimum unsigned big-endian representation.
`A` and `B` always use 384 bytes. All other binary JSON values are lowercase
hex. `counter` is a JSON integer. The code and app name are literal strings.
The two records in each direction contain the existing hello and ping golden
bodies. The tampered record changes the last tag bit of the first app record.

The leading-zero case catches the stable RustCrypto convenience-handshake
padding mismatch. The lower-level SRP arithmetic with explicitly padded u
is required for that version, as described in the protocol.

## TCP verification client

The same test command runs a localhost TCP peer and exercises pairing, hello,
ping, a layout transfer with acknowledgements, widget updates, an event,
keepalive, busy refusal, and reconnect. The peer exists only during the test.

`client.mjs` is also a verification client for the firmware owner. This command
pairs, checks hello and ping, reads the stored landscape layout, updates a
widget if a tile id is supplied, and reconnects without pairing again.

```powershell
node schema/protocol/net-ref/client.mjs <deck-ip> [widget-tile-id]
```

The code prompt hides input. No code or key is accepted as a command-line
argument or printed. The client keeps the credential in memory for this run.
Its pairing occupies a deck slot until `net forget`. A production app uses
the OS keyring. The reference CLI requires a person to read the pairing display.
That board check is separate from this PR's fully automated localhost test.

Automation can import `pairClient` and supply `getCode({signal})` and
`savePending(credential)` callbacks. `savePending` completes before the commit
message. The caller retains that pending credential if the final acknowledgement
is lost, and can pass it to `connectSession`. Neither callback may log secrets.
`connectSession` sends hello, then pings every two seconds until `close()`.
It exposes `request(message)`, `getLayout(orientation)`, and an `onEvent` callback.
An orderly `await session.wire.finish(deadline)` before reconnect avoids racing
the remote socket's close handling. `deadline` uses the exported monotonic `now()`.

The Node client is a verification tool with bounded requests and layout reads.
Its limits and volatile storage are intentional. M3-06 owns the production
transport, transfer engine, discovery, UI, and persistent keyring behavior.

## net-v2

`net-v2.mjs` adds what [net-v2.md](../net-v2.md) changes: item version 2, the
`touchdeck/net-v2/` labels, the X25519 session exchange, `TDN2` records, BLE
fragments and credit values. It imports the SRP arithmetic, hashes, AES-GCM
and link frame checks from `protocol.mjs`. `vectors-v2.mjs` regenerates
[net-v2-vectors.json](../net-v2-vectors.json) from fixed public inputs, and
builds its link bodies from [golden-frames-v2.json](../golden-frames-v2.json).

```powershell
node --test schema/protocol/net-ref/net-v2.test.mjs schema/protocol/net-ref/net-v2-negative.test.mjs
node schema/protocol/net-ref/vectors-v2.mjs --write
```

`name-vectors.mjs` regenerates [name-vectors-v2.json](../name-vectors-v2.json),
the names that link-v2 Device names and net-v2 Companion names share. Its
`\p{Cf}` ranges come from `net-v2.mjs`, and the Firmware host test, the UI
tests and the Companion tests read the same file.

```powershell
node schema/protocol/net-ref/name-vectors.mjs --write
```

Every key, code and identity in those vectors is a synthetic test value. The
test also checks X25519 against RFC 7748's own vectors and HKDF against Node's
built-in implementation. There is no net-v2 socket peer yet; the Firmware and
Companion tickets bring their own.

For per-peer backoff and pairing, `closeOutcome` tells a close the peer starts
from a radio loss, `PeerBackoff.admits` lets a `session.open` whose
`open_tag` matches a stored Pairing past a backoff, and `PairFailures` counts
failed proofs and dropped pairing windows toward the pairing lockout.
`HANGUL_FILLERS` extends the `companion_name` rule.

For discovery privacy, `openTag` and `findPairing` replace the cleartext ids in
`session.open`, `advertisement` builds the TXT record in and out of pairing
mode, and `readRecord` with `matchRecord` is the Companion's side: it finds its
Pairings by hint and drops records it can't match.

`net-v2.mjs` also models three rules the Firmware and Companion must match:
`nameAllowed` with the pinned `FORMAT_RANGES` table for `companion_name`,
`receiveLimit` for the largest BLE fragment a receiver accepts (514 when its
ATT_MTU is unknown), and `PeerBackoff`, which counts only the peer-caused
outcomes in `COUNTED` toward per-peer backoff.

`net-v2-negative.test.mjs` runs every shared negative vector through the
reference: `DeviceSession` and `CompanionSession` answer and check the session
items, `verifyPairProof` checks a pairing proof, `canonicalShare` applies the
X25519 share rule, and `FlowCredit` is the Device's side of BLE credit.

The reference also models three rules that came out of the crypto review.
`RECORD_LIMIT` is `2^24` records per direction per key. `RecordSender` and
`RecordReceiver` refuse the record at the limit and then stay closed, with
`remaining` counting what is left, because the session ends rather than
rekeys. `onHandshakeError` is the Companion's reaction to an unauthenticated
error: `fallback` is always `null` and `versions` is only `[2]`, so a `version`
error never leads to a net-v1 item or a plaintext link. `PairingStore` and
`manage` are the Device's four slots and its USB-only `net.pair`, `net.list`
and `net.forget` messages, with `app_id` removing one Pairing and `all:true`
wiping every one. The `management` vectors replay one ordered run of them.
