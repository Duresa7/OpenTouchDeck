import test from 'node:test';
import assert from 'node:assert/strict';
import { hkdfSync } from 'node:crypto';
import { ascii, bytes, concat, hash512, minimum, extract, validateFrame } from './protocol.mjs';
import {
  derive, handshake, parseHandshake, acceptKey, share, x25519, sessionKeys,
  RecordSender, RecordReceiver, fragment, Reassembler, receiveLimit, creditValue, gatt,
  MAX_ITEM, MIN_INITIAL_CREDIT, FORMAT_RANGES, nameAllowed, PeerBackoff, COUNTED, NOT_COUNTED,
  HANGUL_FILLERS, closeOutcome, PairFailures,
} from './net-v2.mjs';
import { buildNameVectors, readNameVectors, serialize, namesPath } from './name-vectors.mjs';
import { readFileSync } from 'node:fs';
import { inputs, goldenFrames, pairing, session, buildVectors, readVectors } from './vectors-v2.mjs';

const fixture = readVectors();
const pair = pairing();
const sess = session();

function unslip(hex) {
  const wire = bytes(hex);
  const out = [];
  for (let i = 1; i < wire.length - 1; i++) {
    if (wire[i] === 0xdb) out.push(wire[++i] === 0xdc ? 0xc0 : 0xdb);
    else out.push(wire[i]);
  }
  return Buffer.from(out);
}

test('net-v2: reproduce every committed vector without rewriting it', () => {
  assert.deepEqual(buildVectors(), fixture);
  assert.match(fixture.fixture_notice, /synthetic/);
});

test('net-v2: X25519 matches RFC 7748 section 6.1', () => {
  const alice = bytes('77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a');
  const bob = bytes('5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb');
  assert.equal(share(alice).toString('hex'), '8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a');
  assert.equal(share(bob).toString('hex'), 'de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f');
  const shared = '4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742';
  assert.equal(x25519(alice, share(bob)).toString('hex'), shared);
  assert.equal(x25519(bob, share(alice)).toString('hex'), shared);
});

test('net-v2: both sides derive the same session keys', () => {
  const deviceZ = x25519(inputs.deviceEphemeral, sess.clientShare);
  assert.deepEqual(deviceZ, sess.Z);
  const device = sessionKeys({ ltk: inputs.ltk, deviceId: inputs.deviceId, companionId: inputs.companionId,
    clientShare: sess.clientShare, deviceShare: sess.deviceShare, Z: deviceZ });
  assert.deepEqual(device, sess.keys);
});

test('net-v2: HKDF labels match the built-in HKDF with LTK || Z as input keying material', () => {
  const ikm = concat(inputs.ltk, sess.Z);
  for (const label of ['session/deck', 'session/client', 'session/ready', 'session/c2d', 'session/d2c']) {
    const expected = Buffer.from(hkdfSync('sha256', ikm, sess.keys.transcript, ascii(`touchdeck/net-v2/${label}`), 32));
    assert.deepEqual(derive(extract(sess.keys.transcript, ikm), label), expected);
  }
  const pairExpected = Buffer.from(hkdfSync('sha256', pair.client.K, pair.keys.transcript, ascii('touchdeck/net-v2/pair/wrap'), 32));
  assert.deepEqual(pair.keys.wrap, pairExpected);
});

test('net-v2: SRP pairing agrees and delivers the LTK only after the Device proof', () => {
  assert.equal(pair.client.S, pair.server.S);
  assert.deepEqual(hash512(minimum(pair.client.S)), pair.client.K);
  assert.deepEqual(acceptKey(pair.keys, pair.accept), inputs.ltk);
  const forged = Buffer.from(pair.accept);
  forged[4] ^= 1;
  assert.throws(() => acceptKey(pair.keys, forged), /bad deck proof/);
});

test('net-v2: a wrong LTK or a changed share fails the Device proof', () => {
  const base = { ltk: inputs.ltk, deviceId: inputs.deviceId, companionId: inputs.companionId,
    clientShare: sess.clientShare, deviceShare: sess.deviceShare, Z: sess.Z };
  assert.notDeepEqual(sessionKeys({ ...base, ltk: Buffer.alloc(32, 7) }).deckProof, sess.keys.deckProof);
  const otherShare = share(Buffer.alloc(32, 9));
  assert.notDeepEqual(sessionKeys({ ...base, clientShare: otherShare }).deckProof, sess.keys.deckProof);
});

test('net-v2: non-contributory X25519 shares close the handshake', () => {
  assert.throws(() => x25519(inputs.clientEphemeral, bytes(fixture.non_contributory_share.device_share)), /non-contributory/);
  const orderOne = concat(Buffer.from([1]), Buffer.alloc(31));
  assert.throws(() => x25519(inputs.clientEphemeral, orderOne), /non-contributory/);
});

test('net-v2: handshake items carry version 2 and refuse net-v1 items', () => {
  for (const wire of Object.values(fixture.session.wire)) assert.equal(bytes(wire)[3], 2);
  assert.equal(fixture.errors.busy, '00037f0201');
  assert.equal(fixture.errors.closed, '00037f0209');
  assert.throws(() => handshake(0x7f, Buffer.from([10])), /invalid error/);
  const v1 = Buffer.from(bytes(fixture.session.wire.proof));
  v1[3] = 1;
  assert.throws(() => parseHandshake(v1), /version/);
  assert.throws(() => handshake(0x20, Buffer.alloc(53)), /length/);
});

test('net-v2: pair.request names refuse control characters and bidi overrides', () => {
  const request = (name) => {
    const bytesOfName = Buffer.from(name);
    return handshake(0x10, inputs.companionId, Buffer.from([bytesOfName.length]), bytesOfName);
  };
  assert.equal(parseHandshake(request('Desk PC')).kind, 0x10);
  for (const code of [0x00, 0x1b, 0x7f, 0x202a, 0x202e, 0x2066, 0x2069]) {
    assert.throws(() => request(`Desk${String.fromCodePoint(code)}PC`), /control character/);
  }
  assert.equal(parseHandshake(request(`Desk${String.fromCodePoint(0x2065)}PC`)).kind, 0x10);
});

test('net-v2: the shared name vectors accept and refuse the same pair.request items', () => {
  const { accepted, rejected } = fixture.companion_names;
  for (const { label, request } of accepted) assert.equal(parseHandshake(bytes(request), 0x10).kind, 0x10, label);
  for (const { label, request, expected } of rejected) {
    assert.equal(expected, 'close');
    assert.throws(() => parseHandshake(bytes(request), 0x10), /control character/, label);
  }
  const labels = rejected.map(({ label }) => label);
  for (const mark of ['U+200E', 'U+200F', 'U+061C', 'U+200B', 'U+200C', 'U+200D', 'U+2060', 'U+FEFF',
    'U+115F', 'U+1160', 'U+3164', 'U+FFA0']) {
    assert.ok(labels.includes(mark), mark);
  }
});

test('net-v2: the pinned format table is exactly Unicode General_Category Cf', () => {
  const table = new Set();
  for (const [first, last] of FORMAT_RANGES) for (let cp = first; cp <= last; cp++) table.add(cp);
  for (let cp = 0; cp <= 0x10ffff; cp++) {
    if (cp >= 0xd800 && cp <= 0xdfff) continue;
    const character = String.fromCodePoint(cp);
    assert.equal(table.has(cp), /\p{Cf}/u.test(character), `U+${cp.toString(16)}`);
    if (table.has(cp)) assert.equal(nameAllowed(`a${character}b`), false);
  }
  assert.deepEqual(fixture.companion_names.format_ranges, FORMAT_RANGES.map(([first, last]) =>
    (first === last ? first.toString(16).padStart(4, '0')
      : `${first.toString(16).padStart(4, '0')}-${last.toString(16).padStart(4, '0')}`)));
});

test('net-v2: records open each way and carry the golden link-v2 bodies', () => {
  const receivers = { c2d: new RecordReceiver(sess.keys.c2d, 0), d2c: new RecordReceiver(sess.keys.d2c, 1) };
  for (const record of fixture.records) {
    const body = receivers[record.direction].open(bytes(record.wire));
    assert.equal(body.toString('hex'), record.plaintext);
    const golden = goldenFrames.find((entry) => entry.name === record.name);
    if (record.name !== 'hello-selected-response') assert.deepEqual(body, unslip(golden.hex));
  }
  const selected = validateFrame(bytes(fixture.records[3].plaintext));
  const hello = JSON.parse(selected.payload.toString());
  assert.equal(hello.proto, 2);
  assert.equal(hello.id, fixture.inputs.deviceId);
});

test('net-v2: tampered, replayed and reordered records close the receiver', () => {
  const tampered = new RecordReceiver(sess.keys.c2d, 0);
  assert.throws(() => tampered.open(bytes(fixture.tampered_record.wire)));
  assert.throws(() => tampered.open(bytes(fixture.records[0].wire)), /closed/);

  const replay = new RecordReceiver(sess.keys.c2d, 0);
  replay.open(bytes(fixture.records[0].wire));
  assert.throws(() => replay.open(bytes(fixture.records[0].wire)));

  const reorder = new RecordReceiver(sess.keys.c2d, 0);
  assert.throws(() => reorder.open(bytes(fixture.records[2].wire)));

  const sender = new RecordSender(sess.keys.c2d, 0);
  assert.throws(() => new RecordReceiver(sess.keys.c2d, 1).open(sender.seal(bytes(fixture.records[0].plaintext))));
});

test('net-v2: BLE fragments are full size except the last and reassemble to the record', () => {
  for (const record of fixture.records.filter((entry) => entry.ble_fragments)) {
    for (const [key, mtu] of [['att_mtu_23', 23], ['att_mtu_247', 247]]) {
      const parts = record.ble_fragments[key].map(bytes);
      parts.slice(0, -1).forEach((part) => assert.equal(part.length, mtu - 3));
      assert.ok(parts.at(-1).length >= 1 && parts.at(-1).length <= mtu - 3);
      const reassembler = new Reassembler(mtu - 3);
      reassembler.records();
      const items = parts.map((part) => reassembler.push(part)).filter(Boolean);
      assert.deepEqual(items, [bytes(record.wire)]);
    }
  }
  const start = bytes(fixture.pairing.wire.start);
  const handshakeParts = fragment(start, 23);
  assert.equal(handshakeParts.length, Math.ceil(420 / 20));
  const reassembler = new Reassembler(20);
  assert.deepEqual(handshakeParts.map((part) => reassembler.push(part)).filter(Boolean), [start]);
});

test('net-v2: BLE reassembly enforces the handshake and record length bounds', () => {
  const handshakeMax = new Reassembler(244);
  assert.throws(() => handshakeMax.push(Buffer.from([0x02, 0x01])), /item length/);
  assert.throws(() => new Reassembler(244).push(Buffer.from([0x00, 0x02, 0x7f, 0x02])), /item length/);
  assert.equal(new Reassembler(244).push(bytes(fixture.errors.busy)).toString('hex'), fixture.errors.busy);

  const recordMin = new Reassembler(244);
  recordMin.records();
  assert.throws(() => recordMin.push(concat(Buffer.from([0x00, 0x17]), Buffer.alloc(23))), /item length/);
  const recordMax = new Reassembler(244);
  recordMax.records();
  assert.throws(() => recordMax.push(Buffer.from([0x04, 0x19])), /item length/);
  const fullRecord = new Reassembler(244);
  fullRecord.records();
  const maxItem = concat(Buffer.from([0x04, 0x18]), Buffer.alloc(1048));
  assert.deepEqual(fragment(maxItem, 247).map((part) => fullRecord.push(part)).filter(Boolean), [maxItem]);
});

test('net-v2: BLE middle fragments are exactly F and the last is at most F', () => {
  const wire = bytes(fixture.records[3].wire);
  const parts = fragment(wire, 23);

  const shortMiddle = new Reassembler(244);
  shortMiddle.records();
  shortMiddle.push(parts[0]);
  assert.throws(() => shortMiddle.push(parts[1].subarray(0, 19)), /shorter than 20/);

  const changedF = new Reassembler(244);
  changedF.records();
  changedF.push(parts[0]);
  assert.throws(() => changedF.push(concat(parts[1], parts[2])), /differs from F/);

  const tiny = new Reassembler(244);
  tiny.records();
  assert.throws(() => tiny.push(wire.subarray(0, 5)), /shorter than 20/);

  const longLast = new Reassembler(244);
  longLast.records();
  const record = bytes(fixture.records[2].wire);
  longLast.push(record.subarray(0, 20));
  assert.throws(() => longLast.push(record.subarray(20)), /longer than F/);
});

test('net-v2: BLE reassembly refuses fragments that break item boundaries', () => {
  const wire = bytes(fixture.records[2].wire);
  const crossing = new Reassembler(64);
  crossing.records();
  crossing.push(wire.subarray(0, 20));
  assert.throws(() => crossing.push(concat(wire.subarray(20), Buffer.from([0]))), /boundary/);
  assert.throws(() => crossing.push(Buffer.from([0, 0])), /closed/);
  assert.throws(() => new Reassembler(20).push(Buffer.alloc(0)), /fragment size/);
  assert.throws(() => new Reassembler(20).push(Buffer.alloc(21)), /fragment size/);
  assert.throws(() => new Reassembler(20).push(Buffer.from([0])), /prefix/);
  assert.throws(() => new Reassembler(20).push(Buffer.from([0x04, 0x19])), /item length/);
});

test('net-v2: BLE service constants and initial credit', () => {
  assert.equal(new Set([gatt.service, gatt.rx, gatt.tx, gatt.flow]).size, 4);
  assert.equal(MAX_ITEM, 1050);
  assert.ok(fixture.ble.initial_credit >= MIN_INITIAL_CREDIT);
  assert.equal(creditValue(fixture.ble.initial_credit).toString('hex'), fixture.ble.initial_credit_value);
});

test('net-v2: a Companion with an unknown ATT_MTU sends 20-byte fragments and accepts up to 514', () => {
  assert.equal(receiveLimit(), 514);
  assert.equal(receiveLimit(247), 244);
  assert.equal(receiveLimit(23), 20);
  assert.throws(() => receiveLimit(22), /ATT_MTU/);

  const request = bytes(fixture.records[2].wire);
  fragment(request).forEach((part, i, parts) => assert.ok(i === parts.length - 1 ? part.length <= 20 : part.length === 20));

  const response = fixture.records[3];
  const fromDevice = new Reassembler(receiveLimit());
  fromDevice.records();
  const parts = response.ble_fragments.att_mtu_247.map(bytes);
  assert.ok(parts[0].length === 244);
  assert.deepEqual(parts.map((part) => fromDevice.push(part)).filter(Boolean), [bytes(response.wire)]);

  const maxItem = concat(Buffer.from([0x04, 0x18]), Buffer.alloc(1048, 0x5a));
  const largest = new Reassembler(receiveLimit());
  largest.records();
  const largestParts = fragment(maxItem, 517);
  assert.deepEqual(largestParts.map((part) => part.length), [514, 514, 22]);
  assert.deepEqual(largestParts.map((part) => largest.push(part)).filter(Boolean), [maxItem]);

  const tooLarge = new Reassembler(receiveLimit());
  tooLarge.records();
  assert.throws(() => tooLarge.push(maxItem.subarray(0, 515)), /fragment size/);
  const known = new Reassembler(receiveLimit(23));
  known.records();
  assert.throws(() => known.push(parts[0]), /fragment size/);
});

test('net-v2: per-peer backoff doubles on peer-caused failures up to 60 s and resets on session.ready', () => {
  const backoff = new PeerBackoff();
  let now = 0;
  const delays = [];
  for (const outcome of ['deadline', 'auth', 'proof', 'malformed', 'deadline', 'auth', 'proof', 'malformed']) {
    backoff.end('peer-a', outcome, now);
    delays.push(backoff.refusal('peer-a', now));
    now += backoff.refusal('peer-a', now);
    assert.equal(backoff.refusal('peer-a', now), 0);
  }
  assert.deepEqual(delays, [1000, 2000, 4000, 8000, 16000, 32000, 60000, 60000]);
  backoff.end('peer-a', 'ready', now);
  assert.equal(backoff.failures('peer-a'), 0);
  backoff.end('peer-a', 'auth', now);
  assert.equal(backoff.refusal('peer-a', now), 1000);
  assert.throws(() => backoff.end('peer-a', 'mystery', now), /unknown connection outcome/);
});

test('net-v2: a taken-over handshake, busy and a refusal while USB is online never count', () => {
  const backoff = new PeerBackoff();
  // The real Companion keeps starting session.open; a stranger who copied its
  // cleartext companion_id keeps taking the handshake over.
  for (let i = 0; i < 20; i++) {
    backoff.end('companion', 'takeover', i * 100);
    assert.equal(backoff.refusal('companion', i * 100), 0);
  }
  for (let i = 0; i < 20; i++) backoff.end('companion', 'busy', 3000 + i);
  for (let i = 0; i < 20; i++) backoff.end('companion', 'usb-online', 4000 + i);
  assert.equal(backoff.failures('companion'), 0);
  assert.equal(backoff.refusal('companion', 5000), 0);

  // Busy refusals during a backoff don't extend it.
  backoff.end('companion', 'deadline', 10000);
  for (let t = 10000; t < 11000; t += 100) backoff.end('companion', 'busy', t);
  assert.equal(backoff.refusal('companion', 11000), 0);
  backoff.end('companion', 'takeover', 11000);
  backoff.end('companion', 'usb-online', 11000);
  backoff.end('companion', 'deadline', 12000);
  assert.equal(backoff.refusal('companion', 12000), 2000);

  for (const outcome of NOT_COUNTED) {
    const fresh = new PeerBackoff();
    fresh.end('peer', outcome, 0);
    assert.equal(fresh.refusal('peer', 0), 0, outcome);
  }
  assert.deepEqual([...COUNTED].sort(), ['auth', 'deadline', 'malformed', 'peer-closed', 'proof']);
});

test('net-v2: per-peer backoff keeps at least eight peers and evicts the oldest', () => {
  const backoff = new PeerBackoff();
  for (let i = 0; i < 8; i++) backoff.end(`peer-${i}`, 'deadline', i);
  for (let i = 0; i < 8; i++) assert.equal(backoff.failures(`peer-${i}`), 1);
  backoff.end('peer-0', 'auth', 10);
  backoff.end('peer-8', 'deadline', 11);
  assert.equal(backoff.failures('peer-1'), 0);
  assert.equal(backoff.failures('peer-0'), 2);
  assert.equal(backoff.failures('peer-8'), 1);
});

test('net-v2: names refuse the Hangul fillers', () => {
  for (const codePoint of HANGUL_FILLERS) {
    assert.equal(nameAllowed(`Desk${String.fromCodePoint(codePoint)}PC`), false);
    assert.match(String.fromCodePoint(codePoint), /\p{Lo}/u);
  }
  assert.equal(nameAllowed('\ud55c\uae00 PC'), true);
});

test('net-v2: a close the peer starts counts toward backoff, a radio loss does not', () => {
  assert.equal(closeOutcome('tcp', 'fin'), 'peer-closed');
  assert.equal(closeOutcome('tcp', 'rst'), 'peer-closed');
  for (const reason of [0x13, 0x14, 0x15]) assert.equal(closeOutcome('ble', reason), 'peer-closed');
  for (const reason of [0x08, 0x22, 0x3d, 0x3e]) assert.equal(closeOutcome('ble', reason), 'radio-lost');
  assert.throws(() => closeOutcome('tcp', 'timeout'), /unknown/);

  // Connect, idle 1.9 s, drop, repeat: each drop now costs the peer.
  const backoff = new PeerBackoff();
  let now = 0;
  const delays = [];
  for (let i = 0; i < 4; i++) {
    now += 1900;
    backoff.end('idler', closeOutcome('tcp', i % 2 ? 'rst' : 'fin'), now);
    delays.push(backoff.refusal('idler', now));
    now += backoff.refusal('idler', now);
  }
  assert.deepEqual(delays, [1000, 2000, 4000, 8000]);

  const ble = new PeerBackoff();
  for (let i = 0; i < 5; i++) ble.end('flaky', closeOutcome('ble', 0x08), i * 5000);
  assert.equal(ble.failures('flaky'), 0);
  ble.end('flaky', closeOutcome('ble', 0x13), 30000);
  assert.equal(ble.refusal('flaky', 30000), 1000);
});

test('net-v2: a session.open naming a stored companion_id skips per-peer backoff', () => {
  const backoff = new PeerBackoff();
  // An attacker sends from the real Companion's public BLE address with an
  // unknown id, once a minute, keeping that address at the cap.
  for (let minute = 0; minute < 8; minute++) backoff.end('victim', 'auth', minute * 60000);
  const now = 7 * 60000 + 1;
  assert.equal(backoff.refusal('victim', now), 60000 - 1);
  assert.equal(backoff.admits('victim', now, { kind: 0x20, storedCompanion: true }), true);
  assert.equal(backoff.admits('victim', now, { kind: 0x20, storedCompanion: false }), false);
  assert.equal(backoff.admits('victim', now, { kind: 0x10 }), false);
  assert.equal(backoff.admits('victim', now, { kind: 0x10, storedCompanion: true }), false);
  assert.equal(backoff.admits('stranger', now, { kind: 0x10 }), true);
  // The real Companion reaches session.ready, which clears the address.
  backoff.end('victim', 'ready', now + 500);
  assert.equal(backoff.admits('victim', now + 500, { kind: 0x10 }), true);
});

test('net-v2: a dropped pairing window counts toward pair_failures', () => {
  const failures = new PairFailures();
  failures.windowEnded('peer-closed');
  failures.windowEnded('radio-lost');
  assert.equal(failures.lockMs(), 0);
  failures.windowEnded('deadline');
  assert.equal(failures.count, 3);
  assert.equal(failures.lockMs(), 30000);
  failures.windowEnded('malformed');
  assert.equal(failures.lockMs(), 60000);
  for (const outcome of ['cancelled', 'window-expired', 'storage', 'paired']) failures.windowEnded(outcome);
  assert.equal(failures.count, 4);
  assert.throws(() => failures.windowEnded('busy'), /unknown window outcome/);

  // Three failed proofs in one window count three times, not four.
  const proofs = new PairFailures();
  proofs.proofFailed();
  proofs.proofFailed();
  proofs.proofFailed();
  proofs.windowEnded('auth');
  assert.equal(proofs.count, 3);
  proofs.committed();
  assert.equal(proofs.lockMs(), 0);

  const restored = new PairFailures(40);
  assert.equal(restored.lockMs(), 900000);
});

test('names: the shared name vectors match the generator and the net-v2 table', () => {
  const shared = readNameVectors();
  assert.equal(readFileSync(namesPath, 'utf8'), serialize(buildNameVectors()));
  assert.deepEqual(shared.format_ranges, fixture.companion_names.format_ranges);
  assert.deepEqual(shared.hangul_fillers, HANGUL_FILLERS.map((cp) => cp.toString(16)));
  for (const { label, name } of shared.accepted) assert.equal(nameAllowed(name), true, label);
  for (const { label, name } of shared.rejected) assert.equal(nameAllowed(name), false, label);
});
