// Reference for schema/protocol/net-v2.md. It reuses the SRP arithmetic,
// hashes, AES-GCM and link body checks from the net-v1 reference and adds the
// net-v2 item version, labels, the X25519 session exchange, TDN2 records and
// BLE fragmentation. It is a test oracle, not a production crypto backend.
import { createPrivateKey, createPublicKey, diffieHellman } from 'node:crypto';
import {
  ascii, concat, equal, fixed, hash, mac, extract, requireThat, seal, open,
  u16, u64, validateFrame,
} from './protocol.mjs';

export const VERSION = 2;
export const PORT = 47820;
export const MIN_HANDSHAKE_BODY = 3;
export const MAX_HANDSHAKE_BODY = 512;
export const MAX_RECORD_BODY = 1048;
export const MIN_RECORD_BODY = 24;
export const MAX_ITEM = MAX_RECORD_BODY + 2;
export const ATT_MTU_MIN = 23;
export const ATT_MTU_MAX = 517;
export const MIN_INITIAL_CREDIT = MAX_ITEM;

// Records per direction per key. net-v1 allowed 2^32, about 2^38 AES blocks,
// above TLS 1.3's guidance. net-v2 ends the session at 2^24 and never rekeys.
export const RECORD_LIMIT = 1n << 24n;

export const gatt = {
  service: '6198f058-afef-4469-b47f-07315672e2dd',
  rx: '6198f059-afef-4469-b47f-07315672e2dd',
  tx: '6198f05a-afef-4469-b47f-07315672e2dd',
  flow: '6198f05b-afef-4469-b47f-07315672e2dd',
};

export const derive = (prk, label) => mac(prk, ascii(`touchdeck/net-v2/${label}`), Buffer.from([1]));

const SIZES = new Map([
  [0x11, 26], [0x12, 416], [0x13, 432], [0x14, 32], [0x15, 80],
  [0x16, 32], [0x17, 32], [0x20, 54], [0x21, 64], [0x22, 32], [0x23, 32], [0x7f, 1],
]);

// The format characters (General_Category Cf) of Unicode 17.0, as inclusive
// code point ranges. net-v2.md pins this table so implementations without a
// Unicode database agree on it. It covers the bidi marks, overrides and
// isolates, the zero-width characters, the BOM, soft hyphen and tag characters.
export const FORMAT_RANGES = [
  [0x00ad, 0x00ad], [0x0600, 0x0605], [0x061c, 0x061c], [0x06dd, 0x06dd],
  [0x070f, 0x070f], [0x0890, 0x0891], [0x08e2, 0x08e2], [0x180e, 0x180e],
  [0x200b, 0x200f], [0x202a, 0x202e], [0x2060, 0x2064], [0x2066, 0x206f],
  [0xfeff, 0xfeff], [0xfff9, 0xfffb], [0x110bd, 0x110bd], [0x110cd, 0x110cd],
  [0x13430, 0x1343f], [0x1bca0, 0x1bca3], [0x1d173, 0x1d17a], [0xe0001, 0xe0001],
  [0xe0020, 0xe007f],
];

const isFormat = (codePoint) => FORMAT_RANGES.some(([first, last]) => codePoint >= first && codePoint <= last);

// The Hangul fillers are letters (Lo) that render as blank space.
export const HANGUL_FILLERS = [0x115f, 0x1160, 0x3164, 0xffa0];

// A companion_name holds no control (Cc) or format (Cf) characters and no
// Hangul fillers.
export function nameAllowed(name) {
  for (const character of name) {
    const codePoint = character.codePointAt(0);
    if (/\p{Cc}/u.test(character) || isFormat(codePoint) || HANGUL_FILLERS.includes(codePoint)) return false;
  }
  return true;
}

function validateFields(kind, fields) {
  if (kind === 0x10) {
    requireThat(fields.length >= 18 && fields.length <= 81, 'invalid pair request');
    const name = fields.subarray(17);
    requireThat(fields[16] === name.length, 'invalid name length');
    // ignoreBOM keeps a leading U+FEFF in the text, so the check sees it.
    const decoded = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(name);
    requireThat(nameAllowed(decoded), 'control character in name');
    return;
  }
  requireThat(SIZES.has(kind), 'unknown handshake kind');
  requireThat(fields.length === SIZES.get(kind), 'invalid handshake length');
  if (kind === 0x7f) requireThat(fields[0] >= 1 && fields[0] <= 9, 'invalid error');
  if (kind === 0x11) {
    const remaining = fields.readUInt32BE(22);
    requireThat(remaining > 0 && remaining <= 120000, 'invalid window duration');
  }
}

export function handshake(kind, ...parts) {
  const fields = concat(...parts);
  validateFields(kind, fields);
  return concat(u16(fields.length + 2), Buffer.from([kind, VERSION]), fields);
}

export function parseHandshake(wire, expectedKind) {
  requireThat(wire.length >= 4 && wire.length <= MAX_HANDSHAKE_BODY + 2, 'invalid handshake size');
  requireThat(wire.readUInt16BE() === wire.length - 2, 'invalid handshake prefix');
  requireThat(wire[3] === VERSION, 'invalid handshake version');
  const kind = wire[2];
  const fields = wire.subarray(4);
  validateFields(kind, fields);
  if (expectedKind !== undefined) requireThat(kind === expectedKind, 'unexpected handshake kind');
  return { kind, fields };
}

export const errorItem = (reason) => handshake(0x7f, Buffer.from([reason]));

export function pairKeys(K, request, offer, start, challenge) {
  [request, offer, start, challenge].forEach((wire, i) => parseHandshake(wire, 0x10 + i));
  const transcript = hash(ascii('touchdeck/net-v2/pair\0'), request, offer, start, challenge);
  const prk = extract(transcript, fixed(K, 64, 'SRP key'));
  const client = derive(prk, 'pair/client');
  const deck = derive(prk, 'pair/deck');
  const wrap = derive(prk, 'pair/wrap');
  const commit = derive(prk, 'pair/commit');
  const clientProof = mac(client, transcript);
  const deckProof = mac(deck, transcript, clientProof);
  return { transcript, prk, client, deck, wrap, commit, clientProof, deckProof };
}

export function wrapKey(keys, ltk) {
  return seal(keys.wrap, Buffer.alloc(12), fixed(ltk, 32, 'LTK'),
    concat(keys.transcript, keys.clientProof, keys.deckProof));
}

export function acceptKey(keys, acceptance) {
  const { fields } = parseHandshake(acceptance, 0x15);
  requireThat(equal(fields.subarray(0, 32), keys.deckProof), 'bad deck proof');
  return fixed(open(keys.wrap, Buffer.alloc(12), fields.subarray(32),
    concat(keys.transcript, keys.clientProof, keys.deckProof)), 32, 'LTK');
}

export const commitProof = (keys, ltk) => mac(keys.commit, ascii('commit'), keys.transcript, hash(ltk));
export const storedProof = (keys, ltk) => mac(keys.commit, ascii('stored'), keys.transcript, hash(ltk));

// X25519 from RFC 7748 through Node's built-in key objects. The DER prefixes
// wrap a raw 32-byte key in PKCS #8 and SPKI for the X25519 OID 1.3.101.110.
const PKCS8 = Buffer.from('302e020100300506032b656e04220420', 'hex');
const SPKI = Buffer.from('302a300506032b656e032100', 'hex');
const privateKey = (secret) => createPrivateKey({ key: concat(PKCS8, fixed(secret, 32, 'X25519 secret')), format: 'der', type: 'pkcs8' });
const publicKey = (share) => createPublicKey({ key: concat(SPKI, fixed(share, 32, 'X25519 share')), format: 'der', type: 'spki' });

export function share(secret) {
  return createPublicKey(privateKey(secret)).export({ format: 'der', type: 'spki' }).subarray(-32);
}

export const P25519 = (1n << 255n) - 19n;

// net-v2's share rule: a share is canonical, bit 255 clear and u < p, which
// X25519(e, 9) always produces. RFC 7748 masks bit 255 and libraries treat
// u >= p differently, so the check runs before any library sees the share.
export function canonicalShare(peerShare) {
  fixed(peerShare, 32, 'X25519 share');
  requireThat((peerShare[31] & 0x80) === 0, 'non-canonical X25519 share');
  const u = BigInt(`0x${Buffer.from(peerShare).reverse().toString('hex')}`);
  requireThat(u < P25519, 'non-canonical X25519 share');
  return peerShare;
}

// RFC 7748 X25519 without net-v2's checks, for tests only. It returns null
// where OpenSSL refuses the all-zero result.
export function rawX25519(secret, peerShare) {
  try {
    return diffieHellman({ privateKey: privateKey(secret), publicKey: publicKey(peerShare) });
  } catch {
    return null;
  }
}

export function x25519(secret, peerShare) {
  canonicalShare(peerShare);
  const Z = rawX25519(secret, peerShare);
  // OpenSSL refuses an all-zero result itself; treat it like the explicit check.
  requireThat(Z !== null && Z.length === 32 && Z.some((byte) => byte !== 0), 'non-contributory X25519 share');
  return Z;
}

export function sessionKeys({ ltk, deviceId, companionId, clientShare, deviceShare, Z }) {
  const transcript = hash(ascii('touchdeck/net-v2/session\0'), fixed(deviceId, 6),
    fixed(companionId, 16), fixed(clientShare, 32), fixed(deviceShare, 32));
  const prk = extract(transcript, concat(fixed(ltk, 32, 'LTK'), fixed(Z, 32, 'Z')));
  const deck = derive(prk, 'session/deck');
  const client = derive(prk, 'session/client');
  const ready = derive(prk, 'session/ready');
  const c2d = derive(prk, 'session/c2d');
  const d2c = derive(prk, 'session/d2c');
  const deckProof = mac(deck, transcript);
  const clientProof = mac(client, transcript, deckProof);
  const readyProof = mac(ready, transcript, clientProof);
  return { transcript, prk, deck, client, ready, c2d, d2c, deckProof, clientProof, readyProof };
}

// A refusal that sends a handshake error before closing; any other thrown
// error closes the carrier connection without one.
export function refuse(reason) {
  const error = new Error(reason);
  error.reason = reason;
  return error;
}

// The Device's check of a pair.proof: a wrong code gives a different client_proof.
export function verifyPairProof(keys, wire) {
  const { fields } = parseHandshake(wire, 0x14);
  if (!equal(fields, keys.clientProof)) throw refuse('auth');
}

// The Device's side of session setup, with an injected ephemeral secret.
export class DeviceSession {
  constructor({ deviceId, pairings, ephemeral }) {
    this.deviceId = fixed(deviceId, 6, 'device_id');
    this.pairings = pairings;
    this.ephemeral = ephemeral;
    this.keys = null;
  }

  open(wire) {
    const { fields } = parseHandshake(wire, 0x20);
    const companionId = fields.subarray(6, 22);
    const ltk = this.pairings.get(companionId.toString('hex'));
    if (!equal(fields.subarray(0, 6), this.deviceId) || !ltk) throw refuse('auth');
    const clientShare = fields.subarray(22);
    const Z = x25519(this.ephemeral, clientShare);
    const deviceShare = share(this.ephemeral);
    this.keys = sessionKeys({ ltk, deviceId: this.deviceId, companionId, clientShare, deviceShare, Z });
    return handshake(0x21, deviceShare, this.keys.deckProof);
  }

  proof(wire) {
    requireThat(this.keys !== null, 'unexpected session.proof');
    const { fields } = parseHandshake(wire, 0x22);
    requireThat(equal(fields, this.keys.clientProof), 'bad client proof');
    return handshake(0x23, this.keys.readyProof);
  }
}

// The Companion's side of session setup, with an injected ephemeral secret.
export class CompanionSession {
  constructor({ deviceId, companionId, ltk, ephemeral }) {
    Object.assign(this, { deviceId, companionId, ltk, ephemeral, keys: null });
  }

  open() {
    return handshake(0x20, this.deviceId, this.companionId, share(this.ephemeral));
  }

  challenge(wire) {
    const { fields } = parseHandshake(wire, 0x21);
    const deviceShare = fields.subarray(0, 32);
    const Z = x25519(this.ephemeral, deviceShare);
    const keys = sessionKeys({ ltk: this.ltk, deviceId: this.deviceId, companionId: this.companionId,
      clientShare: share(this.ephemeral), deviceShare, Z });
    requireThat(equal(fields.subarray(32), keys.deckProof), 'bad device proof');
    this.keys = keys;
    return handshake(0x22, keys.clientProof);
  }

  ready(wire) {
    requireThat(this.keys !== null, 'unexpected session.ready');
    const { fields } = parseHandshake(wire, 0x23);
    requireThat(equal(fields, this.keys.readyProof), 'bad ready proof');
    return this.keys;
  }
}

export function recordParameters(direction, counter, length) {
  requireThat(direction === 0 || direction === 1, 'invalid direction');
  requireThat(typeof counter === 'bigint' && counter >= 0n && counter < RECORD_LIMIT, 'counter exhausted');
  requireThat(length >= MIN_RECORD_BODY && length <= MAX_RECORD_BODY, 'invalid record length');
  return {
    nonce: concat(Buffer.alloc(4), u64(counter)),
    aad: concat(ascii('TDN2'), Buffer.from([direction]), u64(counter), u16(length)),
  };
}

class Direction {
  constructor(key, direction) {
    this.key = Buffer.from(fixed(key, 32));
    this.direction = direction;
    this.counter = 0n;
    this.closed = false;
  }

  guard(action) {
    requireThat(!this.closed, 'direction closed');
    try {
      return action();
    } catch (error) {
      this.close();
      throw error;
    }
  }

  // Records this direction may still carry. At zero the session must end.
  get remaining() {
    return RECORD_LIMIT - this.counter;
  }

  close() {
    this.closed = true;
    this.key.fill(0);
  }
}

export class RecordSender extends Direction {
  seal(body) {
    return this.guard(() => {
      validateFrame(body);
      const length = body.length + 16;
      const { nonce, aad } = recordParameters(this.direction, this.counter, length);
      const wire = concat(u16(length), seal(this.key, nonce, body, aad));
      this.counter++;
      return wire;
    });
  }
}

export class RecordReceiver extends Direction {
  open(wire) {
    return this.guard(() => {
      requireThat(wire.length >= MIN_RECORD_BODY + 2 && wire.length <= MAX_ITEM, 'invalid record size');
      const length = wire.readUInt16BE();
      requireThat(length === wire.length - 2, 'invalid record prefix');
      const { nonce, aad } = recordParameters(this.direction, this.counter, length);
      const body = open(this.key, nonce, wire.subarray(2), aad);
      validateFrame(body);
      this.counter++;
      return body;
    });
  }
}

// BLE: one item per run of fragments. Every fragment is attMtu - 3 bytes except
// the item's last. attMtu is the negotiated value when the sender knows it, else 23.
export function fragment(item, attMtu = ATT_MTU_MIN) {
  requireThat(Number.isInteger(attMtu) && attMtu >= ATT_MTU_MIN && attMtu <= ATT_MTU_MAX, 'invalid ATT_MTU');
  requireThat(item.length >= 3 && item.length <= MAX_ITEM && item.readUInt16BE() === item.length - 2, 'invalid item');
  const size = attMtu - 3;
  const out = [];
  for (let offset = 0; offset < item.length; offset += size) out.push(item.subarray(offset, offset + size));
  return out;
}

// The largest fragment a receiver accepts: its own ATT_MTU - 3, or 514 when its
// BLE stack doesn't report the MTU, because the peer may have negotiated more.
export function receiveLimit(attMtu = null) {
  if (attMtu === null) return ATT_MTU_MAX - 3;
  requireThat(Number.isInteger(attMtu) && attMtu >= ATT_MTU_MIN && attMtu <= ATT_MTU_MAX, 'invalid ATT_MTU');
  return attMtu - 3;
}

export const PHASE_LIMITS = {
  handshake: [MIN_HANDSHAKE_BODY, MAX_HANDSHAKE_BODY],
  record: [MIN_RECORD_BODY, MAX_RECORD_BODY],
};

// Reassembles BLE fragments into items. The phase sets the allowed body length;
// the caller switches to 'record' after session.ready. The sender's F is learned
// from the first fragment that isn't an item's last, then checked.
export class Reassembler {
  constructor(maxFragment = receiveLimit()) {
    this.maxFragment = maxFragment;
    this.phase = 'handshake';
    this.f = null;
    this.pending = null;
    this.closed = false;
  }

  records() {
    requireThat(this.pending === null, 'phase change inside an item');
    this.phase = 'record';
  }

  push(chunk) {
    requireThat(!this.closed, 'reassembler closed');
    try {
      requireThat(chunk.length >= 1 && chunk.length <= this.maxFragment, 'invalid fragment size');
      if (this.pending === null) {
        requireThat(chunk.length >= 2, 'fragment splits a length prefix');
        const length = chunk.readUInt16BE();
        const [min, max] = PHASE_LIMITS[this.phase];
        requireThat(length >= min && length <= max, 'invalid item length');
        this.pending = { need: length + 2, have: 0, parts: [] };
      }
      const { pending } = this;
      requireThat(pending.have + chunk.length <= pending.need, 'fragment crosses an item boundary');
      const last = pending.have + chunk.length === pending.need;
      if (last) {
        requireThat(this.f === null || chunk.length <= this.f, 'last fragment longer than F');
      } else {
        requireThat(chunk.length >= 20, 'middle fragment shorter than 20');
        if (this.f === null) this.f = chunk.length;
        requireThat(chunk.length === this.f, 'middle fragment differs from F');
      }
      pending.parts.push(Buffer.from(chunk));
      pending.have += chunk.length;
      if (!last) return null;
      this.pending = null;
      return concat(...pending.parts);
    } catch (error) {
      this.closed = true;
      throw error;
    }
  }
}

// Credit on the flow characteristic: a u64be cumulative limit of rx bytes.
export const creditValue = (limit) => u64(BigInt(limit));
export const RECOMMENDED_CREDIT = MIN_INITIAL_CREDIT * 2;

// The Device's side of BLE flow control. The carrier connection starts when
// notifications are on for both tx and flow; the Device then grants its
// initial credit. A write before that, a write beyond the credit, a lower
// limit or turning notifications off ends the carrier connection.
export class FlowCredit {
  constructor(initial = RECOMMENDED_CREDIT) {
    requireThat(Number.isInteger(initial) && initial >= MIN_INITIAL_CREDIT, 'initial credit below one item');
    this.initial = initial;
    this.enabled = new Set();
    this.limit = null;
    this.written = 0;
    this.closed = false;
  }

  guard(action) {
    requireThat(!this.closed, 'carrier connection closed');
    try {
      return action();
    } catch (error) {
      this.closed = true;
      throw error;
    }
  }

  // Returns the initial flow value when this enable starts the carrier connection.
  enable(characteristic) {
    return this.guard(() => {
      requireThat(characteristic === 'tx' || characteristic === 'flow', 'unknown characteristic');
      this.enabled.add(characteristic);
      if (this.limit !== null || this.enabled.size < 2) return null;
      this.limit = this.initial;
      return creditValue(this.limit);
    });
  }

  disable() {
    return this.guard(() => requireThat(false, 'notifications disabled'));
  }

  grant(limit) {
    return this.guard(() => {
      requireThat(this.limit !== null && Number.isInteger(limit) && limit >= this.limit, 'credit decreased');
      this.limit = limit;
      return creditValue(limit);
    });
  }

  write(length) {
    return this.guard(() => {
      requireThat(this.limit !== null, 'write before the carrier connection starts');
      requireThat(this.written + length <= this.limit, 'write beyond credit');
      this.written += length;
    });
  }
}

// Per-peer backoff. Only the outcomes in COUNTED are failures the peer caused.
// 'peer-closed' is a close the peer starts before any terminal error from
// the Device. The others end a connection without counting against its peer:
// a takeover by another session.open, a busy refusal (backoff's own
// included), a session.open refused while USB is online, a radio loss, and
// the person-side endings. 'ready' means the peer reached session.ready.
export const BACKOFF = { first: 1000, cap: 60000, peers: 8 };
export const COUNTED = new Set(['deadline', 'auth', 'proof', 'malformed', 'peer-closed']);
export const NOT_COUNTED = new Set([
  'takeover', 'busy', 'usb-online', 'closed', 'full', 'cancelled', 'storage',
  'window-expired', 'radio-lost', 'paired',
]);

// BLE disconnect reasons a peer sends in its terminate: remote user
// terminated (0x13), low resources (0x14) and power off (0x15). Any other
// reason the Device didn't cause, such as supervision timeout (0x08), is a
// radio loss.
export const BLE_PEER_CLOSE = new Set([0x13, 0x14, 0x15]);

// The outcome of a close the Device didn't start: TCP 'fin' or 'rst', or a
// BLE disconnect reason code.
export function closeOutcome(carrier, reason) {
  if (carrier === 'tcp') {
    requireThat(reason === 'fin' || reason === 'rst', 'unknown TCP close');
    return 'peer-closed';
  }
  requireThat(carrier === 'ble' && Number.isInteger(reason) && reason >= 0 && reason <= 0xff, 'unknown close');
  return BLE_PEER_CLOSE.has(reason) ? 'peer-closed' : 'radio-lost';
}

export class PeerBackoff {
  constructor(peers = BACKOFF.peers) {
    this.peers = peers;
    this.entries = new Map();
  }

  // Milliseconds left in the peer's backoff, 0 when there is none.
  refusal(peer, now) {
    const entry = this.entries.get(peer);
    return entry ? Math.max(0, entry.until - now) : 0;
  }

  // The check on a peer's first handshake item. A session.open naming a
  // stored companion_id skips backoff, as a takeover does; anything else
  // from a peer in backoff gets busy.
  admits(peer, now, { kind, storedCompanion = false } = {}) {
    if (kind === 0x20 && storedCompanion) return true;
    return this.refusal(peer, now) === 0;
  }

  failures(peer) {
    return this.entries.get(peer)?.failures ?? 0;
  }

  end(peer, outcome, now) {
    if (outcome === 'ready') {
      this.entries.delete(peer);
      return;
    }
    if (NOT_COUNTED.has(outcome)) return;
    requireThat(COUNTED.has(outcome), 'unknown connection outcome');
    const failures = this.failures(peer) + 1;
    const delay = Math.min(BACKOFF.first * 2 ** Math.min(failures - 1, 16), BACKOFF.cap);
    this.entries.delete(peer);
    this.entries.set(peer, { failures, until: now + delay });
    if (this.entries.size > this.peers) this.entries.delete(this.entries.keys().next().value);
  }
}

// pair_failures across windows and reboots (kept in NVS on the Device). A
// failed client proof counts. So does a window whose bound carrier connection
// ends any other way before a commit, except Cancel on the Device, the window
// expiring or a storage failure, so dropping a window isn't a free retry.
export const PAIR_LOCK = { first: 30000, cap: 900000, after: 3 };
export const WINDOW_END_FREE = new Set(['cancelled', 'window-expired', 'storage', 'paired']);

export class PairFailures {
  constructor(stored = 0) {
    this.count = stored;
  }

  proofFailed() {
    this.count++;
  }

  // outcome is how the window's carrier connection ended; 'auth' means the
  // third failed proof, which proofFailed already counted.
  windowEnded(outcome) {
    if (outcome === 'auth' || WINDOW_END_FREE.has(outcome)) return;
    requireThat(COUNTED.has(outcome) || outcome === 'radio-lost', 'unknown window outcome');
    this.count++;
  }

  committed() {
    this.count = 0;
  }

  lockMs() {
    if (this.count < PAIR_LOCK.after) return 0;
    return Math.min(PAIR_LOCK.first * 2 ** Math.min(this.count - PAIR_LOCK.after, 16), PAIR_LOCK.cap);
  }
}

// The Companion's reaction to a handshake error. Errors are unauthenticated, so
// nothing an error says can make the Companion accept less than a net-v2
// session: it has no weaker version, no plaintext link and no net-v1 item to
// fall back to. `version` and `closed` are never retried without the person.
export const SESSION_VERSIONS = Object.freeze([VERSION]);

export function onHandshakeError(reason) {
  requireThat(Number.isInteger(reason) && reason >= 1 && reason <= 9, 'invalid error');
  return {
    reason,
    fallback: null,
    versions: SESSION_VERSIONS,
    retry: reason === 2 || reason === 9 ? 'person' : 'rules',
  };
}

// Pairing slots and the USB-only management messages. The store is the
// Device's list; `manage` answers the link messages that read or change it.
export const MAX_PAIRINGS = 4;
export const APP_ID = /^[0-9a-f]{32}$/;
export const FULL_ERROR = 'Four apps are paired. Forget one first.';
export const USB_ONLY_ERROR = 'Only over USB';

export class PairingStore {
  #entries = new Map();

  get count() {
    return this.#entries.size;
  }

  get full() {
    return this.#entries.size >= MAX_PAIRINGS;
  }

  // Never overwrites and never evicts. A full store or a known id is `full`.
  add({ companionId, name, ltk }) {
    const appId = fixed(companionId, 16, 'companion_id').toString('hex');
    if (this.#entries.has(appId) || this.full) throw refuse('full');
    this.#entries.set(appId, { app_id: appId, app_name: name, ltk: Buffer.from(fixed(ltk, 32, 'LTK')) });
  }

  ltk(companionId) {
    return this.#entries.get(Buffer.from(companionId).toString('hex'))?.ltk;
  }

  list() {
    return [...this.#entries.values()].map(({ app_id, app_name }) => ({ app_id, app_name }));
  }

  // Returns how many Pairings were removed, 0 when the id isn't stored.
  forget(appId) {
    const entry = this.#entries.get(appId);
    if (!entry) return 0;
    entry.ltk.fill(0);
    this.#entries.delete(appId);
    return 1;
  }

  forgetAll() {
    const removed = this.#entries.size;
    for (const appId of [...this.#entries.keys()]) this.forget(appId);
    return removed;
  }
}

const failure = (m, reason, error) => ({ m, ok: false, reason, error });

// `carrier` is 'usb', 'wifi' or 'ble'. `base` holds the state fields that the
// Device keeps outside the store, such as `listening` and `pair_failures`.
export function manage(store, request, carrier, base = {}) {
  const m = request.m;
  requireThat(['net.pair', 'net.list', 'net.forget'].includes(m), 'not a management message');
  if (carrier !== 'usb') return failure(m, 'usb-only', USB_ONLY_ERROR);
  const state = () => ({ ...base, paired: store.count });
  if (m === 'net.pair') {
    if (store.full) return failure(m, 'full', FULL_ERROR);
    return { m, ok: true, ...state(), pairing: true };
  }
  if (m === 'net.list') {
    return { m, ok: true, ...state(), free: MAX_PAIRINGS - store.count, pairings: store.list() };
  }
  if (request.app_id !== undefined) {
    if (typeof request.app_id !== 'string' || !APP_ID.test(request.app_id) || request.all !== undefined) {
      return failure(m, 'args', 'Invalid app_id');
    }
    const removed = store.forget(request.app_id);
    return { m, ok: true, ...state(), removed };
  }
  // Wiping every Pairing is explicit, so a request that lost its app_id can't do it.
  if (request.all !== true) return failure(m, 'args', 'Name an app_id, or send all:true');
  const removed = store.forgetAll();
  return { m, ok: true, ...state(), pair_failures: 0, locked_ms: 0, pairing: false, removed };
}
