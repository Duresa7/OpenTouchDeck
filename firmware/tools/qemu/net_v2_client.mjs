// A net-v2 Companion for CI's QEMU check (firmware/tools/qemu_net_check.py), built on the reference in
// schema/protocol/net-ref. It pairs with the QEMU image over TCP, opens a session, exchanges link messages in records,
// and checks the Device's ownership and USB-only rules. It prints `need-code` and reads the six-digit code from stdin,
// which the driver takes from the image's log. Every key it makes is fresh and thrown away.
//
//   node firmware/tools/qemu/net_v2_client.mjs [port]
import { createConnection } from 'node:net';
import { once } from 'node:events';
import { randomBytes } from 'node:crypto';
import { createInterface } from 'node:readline';
import {
  handshake, parseHandshake, pairKeys, acceptKey, commitProof, storedProof, CompanionSession,
  RecordSender, RecordReceiver, MAX_HANDSHAKE_BODY, MAX_RECORD_BODY, MIN_RECORD_BODY,
} from '../../../schema/protocol/net-ref/net-v2.mjs';
import {
  identity, clientPublic, clientSecret, equal, frame, validateFrame, requireThat,
} from '../../../schema/protocol/net-ref/protocol.mjs';

const host = '127.0.0.1';
const port = Number(process.argv[2] ?? 47820);
const REASONS = ['none', 'busy', 'version', 'args', 'full', 'auth', 'expired', 'cancelled', 'storage', 'closed'];

const log = (line) => process.stdout.write(`${line}\n`);

class Wire {
  constructor(socket) {
    this.socket = socket;
    this.socket.setNoDelay(true);
    this.buffer = Buffer.alloc(0);
    this.waiters = [];
    this.ended = false;
    socket.on('data', (data) => { this.buffer = Buffer.concat([this.buffer, data]); this.wake(); });
    socket.on('end', () => { this.ended = true; this.wake(); });
    socket.on('error', () => { this.ended = true; this.wake(); });
  }

  static async connect(deadlineMs = 10000) {
    const socket = createConnection({ host, port });
    const timer = setTimeout(() => socket.destroy(new Error('connect timed out')), deadlineMs);
    try {
      await once(socket, 'connect');
    } finally {
      clearTimeout(timer);
    }
    return new Wire(socket);
  }

  wake() {
    for (const waiter of this.waiters.splice(0)) waiter();
  }

  async exact(size, timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    while (this.buffer.length < size) {
      requireThat(!this.ended, 'connection ended');
      const left = deadline - Date.now();
      requireThat(left > 0, 'read timed out');
      await new Promise((resolve) => {
        const timer = setTimeout(resolve, left);
        this.waiters.push(() => { clearTimeout(timer); resolve(); });
      });
    }
    const out = this.buffer.subarray(0, size);
    this.buffer = this.buffer.subarray(size);
    return out;
  }

  async item(min, max, timeoutMs) {
    const prefix = await this.exact(2, timeoutMs);
    const length = prefix.readUInt16BE();
    requireThat(length >= min && length <= max, `item length ${length} out of bounds`);
    return Buffer.concat([prefix, await this.exact(length, timeoutMs)]);
  }

  send(wire) {
    this.socket.write(wire);
  }

  close() {
    this.socket.destroy();
  }
}

// One handshake item, which must be `kind` and not an error.
async function expect(wire, kind, timeoutMs = 30000) {
  const item = await wire.item(3, MAX_HANDSHAKE_BODY, timeoutMs);
  // An error item is unauthenticated: stop on it, with no fallback and no retry. One with item version 1 is malformed.
  if (item[2] === 0x7f) {
    parseHandshake(item, 0x7f);
    throw new Error(`refused: ${REASONS[item[4]] ?? item[4]}`);
  }
  parseHandshake(item, kind);
  return item;
}

async function connectRetrying(totalMs) {
  const deadline = Date.now() + totalMs;
  for (;;) {
    try {
      return await Wire.connect();
    } catch (error) {
      if (Date.now() > deadline) throw error;
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
  }
}

const stdinLines = createInterface({ input: process.stdin })[Symbol.asyncIterator]();

async function pair(companionId, name) {
  const wire = await connectRetrying(120000);
  try {
    const nameBytes = Buffer.from(name);
    const request = handshake(0x10, companionId, Buffer.from([nameBytes.length]), nameBytes);
    wire.send(request);
    const offer = await expect(wire, 0x11);
    const deviceId = Buffer.from(offer.subarray(4, 10));
    log(`offer device_id=${deviceId.toString('hex')} left_ms=${offer.readUInt32BE(4 + 22)}`);
    log('need-code');
    const line = await stdinLines.next();
    const code = String(line.value ?? '').trim();
    requireThat(/^[0-9]{6}$/.test(code), 'no code');
    const a = randomBytes(32);
    const start = handshake(0x12, clientPublic(a), randomBytes(32));
    wire.send(start);
    const challenge = await expect(wire, 0x13);
    const fields = challenge.subarray(4);
    const secret = clientSecret({ I: identity(deviceId, companionId), code, salt: fields.subarray(0, 16), a,
      B: fields.subarray(16, 400) });
    const keys = pairKeys(secret.K, request, offer, start, challenge);
    wire.send(handshake(0x14, keys.clientProof));
    const accept = await expect(wire, 0x15);
    const ltk = acceptKey(keys, accept);
    wire.send(handshake(0x16, commitProof(keys, ltk)));
    const stored = await expect(wire, 0x17);
    requireThat(equal(stored.subarray(4), storedProof(keys, ltk)), 'bad stored proof');
    a.fill(0);
    log('paired');
    return { deviceId, ltk };
  } finally {
    wire.close();
  }
}

class Session {
  constructor(wire, keys) {
    this.wire = wire;
    this.sender = new RecordSender(keys.c2d, 0);
    this.receiver = new RecordReceiver(keys.d2c, 1);
    this.seq = 0;
  }

  async request(message) {
    this.seq = this.seq % 255 + 1;
    const seq = this.seq;
    this.wire.send(this.sender.seal(frame(message, { seq })));
    for (;;) {
      const record = await this.wire.item(MIN_RECORD_BODY, MAX_RECORD_BODY, 30000);
      const decoded = validateFrame(this.receiver.open(record));
      if (decoded.type !== 1 || decoded.flags !== 1 || decoded.seq !== seq) continue;
      return JSON.parse(decoded.payload.toString('utf8'));
    }
  }
}

async function session(deviceId, companionId, ltk) {
  const wire = await connectRetrying(10000);
  const companion = new CompanionSession({ deviceId, companionId, ltk, ephemeral: randomBytes(32) });
  wire.send(companion.open());
  wire.send(companion.challenge(await expect(wire, 0x21)));
  const keys = companion.ready(await expect(wire, 0x23));
  log('session ready');
  return new Session(wire, keys);
}

async function main() {
  const companionId = randomBytes(16);
  const { deviceId, ltk } = await pair(companionId, 'QEMU check');
  const link = await session(deviceId, companionId, ltk);

  const hello = await link.request({ m: 'hello' });
  log(`hello ${JSON.stringify({ ok: hello.ok, product: hello.product, proto: hello.proto, carrier: hello.carrier })}`);
  requireThat(hello.ok === true && hello.product === 'touchdeck' && hello.carrier === 'wifi', 'hello over wifi');

  const state = await link.request({ m: 'link.state' });
  log(`link.state ${JSON.stringify({ online: state.online, carrier: state.carrier })}`);
  requireThat(state.ok === true && state.online === true && state.carrier === 'wifi', 'link online over wifi');

  const net = await link.request({ m: 'net.state' });
  log(`net.state ${JSON.stringify({ paired: net.paired, session: net.session })}`);
  requireThat(net.ok === true && net.paired === 1 && net.session?.app_name === 'QEMU check'
    && net.session?.app_id === companionId.toString('hex') && net.session?.carrier === 'wifi', 'net.state session');

  for (const m of ['net.pair', 'net.list', 'net.forget', 'update.begin']) {
    const refused = await link.request({ m });
    requireThat(refused.ok === false && refused.reason === 'usb-only', `${m} refused as usb-only`);
  }
  log('usb-only refused');

  // A proven owner holds the Device: a second connection gets busy at once.
  const second = await Wire.connect();
  const busy = await second.item(3, MAX_HANDSHAKE_BODY, 5000);
  second.close();
  requireThat(busy.toString('hex') === '00037f0201', `second connection got ${busy.toString('hex')}`);
  log('second connection busy');

  const ping = await link.request({ m: 'ping', n: 7 });
  requireThat(ping.ok === true && ping.n === 7, 'ping after busy');
  link.wire.close();
  log('net-v2 qemu check passed');
}

main().then(() => process.exit(0), (error) => {
  log(`net-v2 qemu check failed: ${error.message}`);
  process.exit(1);
});
