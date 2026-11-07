// Regenerates schema/protocol/net-v2-vectors.json from public, deterministic
// inputs. Every key and secret here is a synthetic test value. Never use one on
// a real Device. Run from the repository root:
//   node schema/protocol/net-ref/vectors-v2.mjs --write
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import {
  bytes, concat, minimum, identity, clientPublic, clientSecret, serverSecret, u16, u32, u64, frame,
  seal,
} from './protocol.mjs';
import {
  gatt, handshake, errorItem, pairKeys, wrapKey, commitProof, storedProof,
  share, x25519, sessionKeys, recordParameters, RecordSender, fragment, creditValue,
  MIN_INITIAL_CREDIT, FORMAT_RANGES, VERSION, RECORD_LIMIT, PairingStore, manage, onHandshakeError, MAX_HANDSHAKE_BODY, MAX_RECORD_BODY, MIN_RECORD_BODY,
  openTag, derive, discoveryKey, hint, advertisement,
} from './net-v2.mjs';

const ramp = (start, length) => Buffer.from(Array.from({ length }, (_, i) => (start + i) & 255));
const hex = (value) => (typeof value === 'bigint' ? minimum(value) : value).toString('hex');
const hexObject = (values) => Object.fromEntries(Object.entries(values).map(([key, value]) => [key, hex(value)]));

export const inputs = {
  deviceId: bytes('020000000001'), companionId: ramp(0x10, 16), companionName: 'Vector workstation',
  code: '123456', salt: ramp(0x30, 16), a: ramp(0x50, 32), b: ramp(0x70, 32),
  windowId: ramp(0x90, 16), clientNonce: ramp(0xa0, 32), deckNonce: ramp(0xc0, 32),
  ltk: ramp(0xe0, 32), clientEphemeral: ramp(0x01, 32), deviceEphemeral: ramp(0x21, 32),
  discoveryNonce: ramp(0x5a, 16), deviceName: 'Vector deck',
};

// The three random fillers of the fixture's free slots, and another record's.
export const discoveryInputs = {
  fillers: [ramp(0x70, 8), ramp(0x78, 8), ramp(0x80, 8)],
  otherNonce: ramp(0x9a, 16),
  otherFillers: [ramp(0xa0, 8), ramp(0xa8, 8), ramp(0xb0, 8), ramp(0xb8, 8)],
};

export const goldenFrames = JSON.parse(readFileSync(new URL('../golden-frames-v2.json', import.meta.url), 'utf8')).frames;

// The link body of a golden-frames-v2 fixture, rebuilt from its decoded fields.
export function goldenBody(name, override) {
  const fixture = goldenFrames.find((entry) => entry.name === name);
  if (!fixture) throw new Error(`missing golden frame ${name}`);
  const { type, seq, flags, payload_hex: payloadHex } = fixture.decoded;
  const payload = override ? Buffer.from(JSON.stringify({ ...fixture.message, ...override })) : bytes(payloadHex);
  return frame(payload, { type, seq, flags });
}

export function pairing(fixture = inputs) {
  const I = identity(fixture.deviceId, fixture.companionId);
  const name = Buffer.from(fixture.companionName);
  const request = handshake(0x10, fixture.companionId, Buffer.from([name.length]), name);
  const offer = handshake(0x11, fixture.deviceId, fixture.windowId, u32(120000));
  const A = clientPublic(fixture.a);
  const start = handshake(0x12, A, fixture.clientNonce);
  const server = serverSecret({ I, code: fixture.code, salt: fixture.salt, b: fixture.b, A });
  const client = clientSecret({ I, code: fixture.code, salt: fixture.salt, a: fixture.a, B: server.B });
  const challenge = handshake(0x13, fixture.salt, server.B, fixture.deckNonce);
  const keys = pairKeys(client.K, request, offer, start, challenge);
  const wrapped = wrapKey(keys, fixture.ltk);
  const proof = handshake(0x14, keys.clientProof);
  const accept = handshake(0x15, keys.deckProof, wrapped);
  const commit = handshake(0x16, commitProof(keys, fixture.ltk));
  const stored = handshake(0x17, storedProof(keys, fixture.ltk));
  return { I, client, server, keys, wrapped, request, offer, start, challenge, proof, accept, commit, stored };
}

export function session(fixture = inputs) {
  const clientShare = share(fixture.clientEphemeral);
  const deviceShare = share(fixture.deviceEphemeral);
  const Z = x25519(fixture.clientEphemeral, deviceShare);
  const keys = sessionKeys({ ltk: fixture.ltk, deviceId: fixture.deviceId, companionId: fixture.companionId,
    clientShare, deviceShare, Z });
  const wire = {
    open: handshake(0x20, openTag(fixture.ltk, clientShare), clientShare),
    challenge: handshake(0x21, deviceShare, keys.deckProof),
    proof: handshake(0x22, keys.clientProof),
    ready: handshake(0x23, keys.readyProof),
  };
  const openKey = derive(fixture.ltk, 'session/open');
  return { clientShare, deviceShare, Z, keys, wire, openKey, openTag: openTag(fixture.ltk, clientShare) };
}

export function discovery(fixture = inputs) {
  const out = advertisement({ nonce: fixture.discoveryNonce, ltks: [fixture.ltk], fillers: discoveryInputs.fillers });
  const pairingMode = advertisement({ nonce: fixture.discoveryNonce, ltks: [fixture.ltk], fillers: discoveryInputs.fillers,
    pairing: { deviceId: fixture.deviceId, name: fixture.deviceName, paired: 1 } });
  const other = advertisement({ nonce: discoveryInputs.otherNonce, ltks: [], fillers: discoveryInputs.otherFillers });
  const key = discoveryKey(fixture.ltk);
  return {
    notice: 'Public synthetic test values only.',
    K_disc: hex(key), nonce: hex(fixture.discoveryNonce), fillers: discoveryInputs.fillers.map(hex),
    hint: hex(hint(key, fixture.discoveryNonce)), slot: 0,
    label: out.label, instance: out.instance, host: out.host,
    txt: out.txt, pairing_txt: pairingMode.txt,
    other_record: { note: 'another nonce and only fillers: it matches no Pairing', txt: other.txt },
  };
}

export function recordBodies() {
  return [
    { name: 'hello-discovery-request', direction: 0, body: goldenBody('hello-discovery-request') },
    { name: 'hello-discovery-response', direction: 1, body: goldenBody('hello-discovery-response') },
    { name: 'hello-selected-request', direction: 0, body: goldenBody('hello-selected-request') },
    { name: 'hello-selected-response', direction: 1,
      body: goldenBody('hello-selected-response', { id: inputs.deviceId.toString('hex') }) },
  ];
}

// A pair.request built without the name check, so rejected names can be encoded.
export function rawRequest(name, companionId = inputs.companionId) {
  const encoded = Buffer.from(name, 'utf8');
  return concat(u16(encoded.length + 19), Buffer.from([0x10, VERSION]), companionId,
    Buffer.from([encoded.length]), encoded);
}

const hex4 = (codePoint) => codePoint.toString(16).padStart(4, '0');
const codePointLabel = (codePoint) => `U+${hex4(codePoint).toUpperCase()}`;
const rangeLabel = ([first, last]) => (first === last ? hex4(first) : `${hex4(first)}-${hex4(last)}`);

// Names every implementation MUST accept, and names with one control or format
// character that it MUST refuse by closing the carrier connection.
export const acceptedNames = [
  ['ascii', 'Desk PC'],
  ['accented', 'Bureau d\u2019\u00e9t\u00e9'],
  ['right-to-left letters', '\u05de\u05d7\u05e9\u05d1'],
  ['emoji with variation selector', 'Desk \u{1f5a5}\ufe0f'],
  ['unassigned neighbour U+2065', 'Desk\u2065PC'],
];
export const rejectedCodePoints = [
  0x0000, 0x001b, 0x007f, 0x0085, 0x00ad, 0x061c, 0x115f, 0x1160, 0x180e, 0x200b, 0x200c,
  0x200d, 0x200e, 0x200f, 0x202a, 0x202b, 0x202c, 0x202d, 0x202e, 0x2060, 0x2066, 0x2067,
  0x2068, 0x2069, 0x3164, 0xfeff, 0xffa0, 0xe0001, 0xe0041,
];

export function companionNames() {
  return {
    format_ranges: FORMAT_RANGES.map(rangeLabel),
    accepted: acceptedNames.map(([label, name]) => ({ label, request: hex(rawRequest(name)) })),
    rejected: [
      ...rejectedCodePoints.map((codePoint) => ({ label: codePointLabel(codePoint),
        request: hex(rawRequest(`Desk${String.fromCodePoint(codePoint)}PC`)), expected: 'close' })),
      { label: 'leading U+FEFF', request: hex(rawRequest('\ufeffDesk PC')), expected: 'close' },
    ],
  };
}

// Every small-order X25519 u-coordinate, in its canonical form and as the
// non-canonical encodings u + p, some with bit 255 set (from cr.yp.to/ecdh.html).
export const smallOrderShares = [
  ['u = 0', '0000000000000000000000000000000000000000000000000000000000000000'],
  ['u = 1', '0100000000000000000000000000000000000000000000000000000000000000'],
  ['small order', 'e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800'],
  ['small order', '5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157'],
  ['u = p - 1', 'ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f'],
  ['u = p, non-canonical 0', 'edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f'],
  ['u = p + 1, non-canonical 1', 'eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f'],
  ['small order + p, bit 255 set', 'cdeb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b880'],
  ['small order + p, bit 255 set', '4c9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f11d7'],
  ['u = 2p - 1, bit 255 set', 'd9ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'],
  ['u = 2p, bit 255 set', 'daffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'],
  ['u = 2p + 1, bit 255 set', 'dbffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'],
];

// Synthetic values used only by the negative vectors.
export const negativeInputs = {
  wrongCode: '654321',
  otherEphemeral: ramp(0x41, 32),
  unknownLtk: ramp(0x60, 32),
};

const flip = (wire, index) => {
  const out = Buffer.from(wire);
  out[index] ^= 1;
  return out;
};

// A record sealed at any counter, past the limit included, so receivers can
// prove they refuse it even though its tag is valid.
export function sealAt(key, direction, counter, body) {
  const length = body.length + 16;
  const aad = concat(Buffer.from('TDN2', 'ascii'), Buffer.from([direction]), u64(counter), u16(length));
  return concat(u16(length), seal(key, concat(Buffer.alloc(4), u64(counter)), body, aad));
}

export function negativeVectors(pair = pairing(), sess = session()) {
  const wrong = clientSecret({ I: pair.I, code: negativeInputs.wrongCode, salt: inputs.salt, a: inputs.a, B: pair.server.B });
  const wrongKeys = pairKeys(wrong.K, pair.request, pair.offer, pair.start, pair.challenge);

  const otherShare = share(negativeInputs.otherEphemeral);
  const swappedOpen = handshake(0x20, sess.openTag, otherShare);
  const tamperedTag = Buffer.from(sess.openTag);
  tamperedTag[0] ^= 1;

  const v1Error = Buffer.from(errorItem(2));
  v1Error[3] = 1;
  const highBit = Buffer.from(sess.deviceShare);
  highBit[31] |= 0x80;

  const ping = frame({ m: 'ping' });
  const maxBody = frame(Buffer.alloc(1024, 0x5a), { type: 2 });
  const last = RECORD_LIMIT - 1n;
  const credit = MIN_INITIAL_CREDIT * 2;

  return {
    notice: 'Every implementation MUST refuse these inputs. Public synthetic test values only.',
    inputs: { wrong_code: negativeInputs.wrongCode, other_ephemeral: hex(negativeInputs.otherEphemeral),
      unknown_ltk: hex(negativeInputs.unknownLtk) },
    wrong_code: { receiver: 'device', note: 'pairing.wire request, offer, start and challenge, then this proof',
      proof: hex(handshake(0x14, wrongKeys.clientProof)), expected: 'auth', error: hex(errorItem(5)) },
    tampered_wrapped_key: [
      { receiver: 'companion', note: 'first ciphertext byte flipped', accept: hex(flip(pair.accept, 4 + 32)), expected: 'close' },
      { receiver: 'companion', note: 'last tag byte flipped', accept: hex(flip(pair.accept, pair.accept.length - 1)), expected: 'close' },
    ],
    swapped_client_share: { receiver: 'device',
      note: 'session.open carries another client_share under the original open_tag',
      open: hex(swappedOpen), expected: 'auth', error: hex(errorItem(5)) },
    swapped_device_share: { receiver: 'companion',
      note: 'session.challenge carries another device_share with the original device_proof',
      challenge: hex(handshake(0x21, otherShare, sess.keys.deckProof)), expected: 'close' },
    unknown_open_tag: { receiver: 'device', note: 'tagged with inputs.unknown_ltk, which the Device does not hold',
      open: hex(handshake(0x20, openTag(negativeInputs.unknownLtk, sess.clientShare), sess.clientShare)),
      expected: 'auth', error: hex(errorItem(5)) },
    tampered_open_tag: { receiver: 'device', note: 'the fixture session.open with the first tag bit flipped',
      open: hex(handshake(0x20, tamperedTag, sess.clientShare)), expected: 'auth', error: hex(errorItem(5)) },
    counter_limit: { direction: 'c2d', plaintext: hex(ping), limit: Number(RECORD_LIMIT), at_limit: 'end the session',
      last_counter: Number(last), last: hex(sealAt(sess.keys.c2d, 0, last, ping)), last_expected: 'accept',
      beyond_counter: Number(RECORD_LIMIT), beyond: hex(sealAt(sess.keys.c2d, 0, RECORD_LIMIT, ping)), beyond_expected: 'close' },
    max_record: { direction: 'c2d', counter: 0, plaintext: hex(maxBody),
      wire: hex(new RecordSender(sess.keys.c2d, 0).seal(maxBody)), expected: 'accept' },
    oversize_items: [
      { phase: 'handshake', prefix: hex(u16(MAX_HANDSHAKE_BODY + 1)), expected: 'close' },
      { phase: 'record', prefix: hex(u16(MAX_RECORD_BODY + 1)), expected: 'close' },
      { phase: 'record', prefix: hex(u16(MIN_RECORD_BODY - 1)), expected: 'close' },
    ],
    write_before_credit: { initial_credit: credit,
      events: [{ enable: 'flow' }, { write: 20 }], expected: 'close' },
    credit_overrun: { initial_credit: credit,
      events: [{ enable: 'tx' }, { enable: 'flow' }, { write: 20, repeat: credit / 20 }, { write: 1 }],
      accepted_bytes: credit, expected: 'close' },
    version_error: { receiver: 'companion', error: hex(errorItem(2)), expected: 'stop',
      note: 'An unauthenticated hint. The Companion sends no net-v1 item, no plaintext link and no retry of its own.',
      outcome: onHandshakeError(2),
      v1_item: { note: 'the same error item with item version 1', wire: hex(v1Error), expected: 'close' } },
    x25519: {
      rule: 'A share with bit 255 set or u >= p closes before X25519. Otherwise a share the X25519 library refuses, or an all-zero Z, closes.',
      secret: hex(inputs.clientEphemeral),
      small_order: smallOrderShares.map(([note, value]) => ({ note, share: value, expected: 'close' })),
      bit_255_set: { note: 'session.device_share with bit 255 set; masking alone would give session.Z',
        share: hex(highBit), expected: 'close' },
    },
  };
}

// Four synthetic apps fill the store, then each request runs in order.
export function managementVectors() {
  const apps = [0x10, 0x20, 0x30, 0x40].map((start, i) => ({ companionId: ramp(start, 16),
    name: `Vector app ${i + 1}`, ltk: ramp(0xe0 + i, 32) }));
  const store = new PairingStore();
  apps.forEach((app) => store.add(app));
  const base = { listening: true, port: 47820, ble: false, session: null, pairing: false, pair_failures: 1, locked_ms: 0 };
  const second = apps[1].companionId.toString('hex');
  const steps = [
    ['net.list over USB shows four Pairings', { m: 'net.list' }, 'usb'],
    ['net.pair over USB with four Pairings is full', { m: 'net.pair' }, 'usb'],
    ['net.forget with an app_id removes one Pairing', { m: 'net.forget', app_id: second }, 'usb'],
    ['the same request again removes none and still succeeds', { m: 'net.forget', app_id: second }, 'usb'],
    ['net.pair over USB opens a window now that a slot is free', { m: 'net.pair' }, 'usb'],
    ['net.forget over Wi-Fi is USB-only', { m: 'net.forget', app_id: apps[0].companionId.toString('hex') }, 'wifi'],
    ['net.list over BLE is USB-only', { m: 'net.list' }, 'ble'],
    ['net.forget with no app_id and no all is refused', { m: 'net.forget' }, 'usb'],
    ['net.forget with an app_id that is not 32 lowercase hex is refused', { m: 'net.forget', app_id: second.toUpperCase() }, 'usb'],
    ['net.forget with all:true wipes the rest and clears the lockout', { m: 'net.forget', all: true }, 'usb'],
  ];
  return {
    apps: apps.map((app) => ({ app_id: hex(app.companionId), app_name: app.name })),
    base,
    steps: steps.map(([note, request, carrier]) => ({ note, carrier, request, response: manage(store, request, carrier, base) })),
  };
}

export function buildVectors() {
  const pair = pairing();
  const sess = session();
  const senders = [new RecordSender(sess.keys.c2d, 0), new RecordSender(sess.keys.d2c, 1)];
  const counters = [0n, 0n];
  const records = recordBodies().map(({ name, direction, body }) => {
    const counter = counters[direction]++;
    const { nonce, aad } = recordParameters(direction, counter, body.length + 16);
    const wire = senders[direction].seal(body);
    const entry = { name, direction: direction === 0 ? 'c2d' : 'd2c', counter: Number(counter),
      plaintext: hex(body), nonce: hex(nonce), aad: hex(aad), wire: hex(wire) };
    if (name.startsWith('hello-selected')) {
      entry.ble_fragments = {
        att_mtu_23: fragment(wire, 23).map(hex),
        att_mtu_247: fragment(wire, 247).map(hex),
      };
    }
    return entry;
  });
  const tampered = bytes(records[0].wire);
  tampered[tampered.length - 1] ^= 1;
  return {
    protocol: 'touchdeck/net-v2',
    fixture_notice: 'Public synthetic test values only. Never use these keys, codes or identities on a real Device.',
    suite: 'SRP-6a-3072-SHA512/X25519/HKDF-SHA256/AES-256-GCM',
    inputs: { ...hexObject(Object.fromEntries(Object.entries(inputs).filter(([, v]) => Buffer.isBuffer(v)))),
      code: inputs.code, companionName: inputs.companionName, deviceName: inputs.deviceName },
    pake: { identity: pair.I.toString('ascii'), ...hexObject({ A: pair.client.A, B: pair.server.B,
      u: pair.client.u, S: pair.client.S, K_srp: pair.client.K }) },
    pairing: { ...hexObject(pair.keys), wrapped_key: hex(pair.wrapped),
      wire: hexObject(Object.fromEntries(['request', 'offer', 'start', 'challenge', 'proof', 'accept', 'commit', 'stored']
        .map((name) => [name, pair[name]]))) },
    session: { ...hexObject({ client_share: sess.clientShare, device_share: sess.deviceShare, Z: sess.Z,
      K_open: sess.openKey, open_tag: sess.openTag }),
      ...hexObject(sess.keys), wire: hexObject(sess.wire) },
    discovery: discovery(),
    records,
    tampered_record: { direction: 'c2d', counter: 0, wire: hex(tampered), expected: 'close' },
    non_contributory_share: { device_share: hex(Buffer.alloc(32)), expected: 'close' },
    companion_names: companionNames(),
    negative: negativeVectors(pair, sess),
    errors: { busy: hex(errorItem(1)), version: hex(errorItem(2)), closed: hex(errorItem(9)) },
    management: managementVectors(),
    ble: { ...gatt, initial_credit: MIN_INITIAL_CREDIT * 2, initial_credit_value: hex(creditValue(MIN_INITIAL_CREDIT * 2)) },
  };
}

export const vectorPath = new URL('../net-v2-vectors.json', import.meta.url);
export const readVectors = () => JSON.parse(readFileSync(vectorPath, 'utf8'));

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv[2] !== '--write') throw new Error('Use --write to regenerate public test fixtures');
  writeFileSync(vectorPath, `${JSON.stringify(buildVectors(), null, 2)}\n`);
  process.stdout.write('Wrote net-v2-vectors.json with public synthetic test values.\n');
}
