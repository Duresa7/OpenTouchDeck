import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash, createPublicKey, randomBytes, verify } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { blake2b, generateKeyPair, readSecretKey, scryptParams, signFile } from '../release-signer/minisign.mjs';

// Run with: node --test .github/release-signer-test/signer.test.mjs
const CLI = fileURLToPath(new URL('../release-signer/cli.mjs', import.meta.url));
const tauri = JSON.parse(await readFile(new URL('./fixtures/tauri-cli-2.12.4-test-only-key.json', import.meta.url), 'utf8'));
const run = promisify(execFile);
const box = wrapped => Buffer.from(wrapped, 'base64').toString('utf8').split('\n');

async function folder(t) {
  const dir = await mkdtemp(join(tmpdir(), 'touchdeck-signer-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  return dir;
}

async function fixtureFile(t) {
  const path = join(await folder(t), tauri.file);
  await writeFile(path, tauri.data);
  return path;
}

// An independent check of a signature, as a Tauri updater makes it: the prehashed (ED) file signature, then the
// global signature over that signature and the trusted comment. Returns the trusted comment.
async function verifyInstaller(path, wrappedSignature, wrappedKey) {
  const key = Buffer.from(box(wrappedKey)[1], 'base64');
  const lines = box(wrappedSignature);
  const packet = Buffer.from(lines[1], 'base64');
  const global = Buffer.from(lines[3], 'base64');
  assert.equal(key.length, 42);
  assert.equal(packet.length, 74);
  assert.equal(global.length, 64);
  assert.equal(packet.subarray(0, 2).toString(), 'ED');
  assert.deepEqual(packet.subarray(2, 10), key.subarray(2, 10));
  assert.ok(lines[2].startsWith('trusted comment: '));
  const publicKey = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key.subarray(10)]), format: 'der', type: 'spki' });
  const signature = packet.subarray(10);
  const comment = lines[2].slice(17);
  if (!verify(null, Buffer.concat([signature, Buffer.from(comment)]), publicKey, global)) throw new Error('Invalid comment signature');
  const digest = createHash('blake2b512').update(await readFile(path)).digest();
  if (!verify(null, digest, publicKey, signature)) throw new Error('Invalid installer signature');
  return comment;
}

test('BLAKE2b matches node:crypto at 64 bytes and the reference vectors at 32', () => {
  for (const length of [0, 1, 127, 128, 129, 256, 1000]) {
    const data = randomBytes(length);
    assert.deepEqual(blake2b(data, 64), createHash('blake2b512').update(data).digest(), `length ${length}`);
  }
  assert.equal(blake2b(Buffer.alloc(0), 32).toString('hex'), '0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8');
  assert.equal(blake2b(Buffer.from('abc'), 32).toString('hex'), 'bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319');
});

test('scrypt cost for the minisign default limits is N=2^15, r=8, p=1', () => {
  assert.deepEqual(scryptParams(1_048_576n, 33_554_432n), { N: 32768, r: 8, p: 1, maxmem: 256 * 32768 * 8 });
  assert.throws(() => scryptParams(1n << 40n, 1n << 31n), /too much scrypt memory/);
});

test('a Tauri CLI key decrypts to the same public key the CLI wrote, and a wrong password is refused', () => {
  assert.equal(readSecretKey(tauri.privateKey, tauri.password).publicKeyBox, tauri.publicKey);
  assert.throws(() => readSecretKey(tauri.privateKey, 'not the password'), /Wrong password/);
});

test('the Tauri CLI signature still verifies', async t => {
  const comment = await verifyInstaller(await fixtureFile(t), tauri.signature, tauri.publicKey);
  assert.match(comment, new RegExp(`\\tfile:${tauri.file}\\tversion:${tauri.version}$`));
});

test('signing with a Tauri CLI key matches the CLI output in every field but the hedged signatures', async t => {
  const timestamp = Number(/^trusted comment: timestamp:(\d+)\t/.exec(box(tauri.signature)[2])[1]);
  const ours = await signFile(await fixtureFile(t), readSecretKey(tauri.privateKey, tauri.password), tauri.version, { timestamp });
  const [cli, mine] = [box(tauri.signature), box(ours)];
  assert.equal(ours.length, tauri.signature.length);
  assert.doesNotMatch(ours, /\s/);
  assert.equal(mine.length, 5);
  assert.equal(mine[4], '');
  assert.equal(mine[0], cli[0]);
  assert.equal(mine[2], cli[2]);
  const [cliPacket, myPacket] = [Buffer.from(cli[1], 'base64'), Buffer.from(mine[1], 'base64')];
  assert.deepEqual(myPacket.subarray(0, 10), cliPacket.subarray(0, 10));
  assert.equal(myPacket.length, 74);
  assert.equal(Buffer.from(mine[3], 'base64').length, 64);
  assert.equal([cli[0], mine[1], cli[2], mine[3], ''].join('\n'), mine.join('\n'));
});

test('a signed release verifies, and a wrong version, a flipped byte or another key does not', async t => {
  const data = randomBytes(200_000);
  const path = join(await folder(t), 'OpenTouchDeck-windows-x64-setup.exe');
  await writeFile(path, data);
  const signature = await signFile(path, readSecretKey(tauri.privateKey, tauri.password), '1.2.3');
  assert.match(await verifyInstaller(path, signature, tauri.publicKey), /\tversion:1\.2\.3$/);
  assert.doesNotMatch(await verifyInstaller(path, signature, tauri.publicKey), /\tversion:1\.2\.4$/);
  await assert.rejects(verifyInstaller(path, signature, generateKeyPair().publicKey));
  const flipped = Buffer.from(data); flipped[flipped.length >> 1] ^= 1;
  await writeFile(path, flipped);
  await assert.rejects(verifyInstaller(path, signature, tauri.publicKey), /Invalid installer signature/);
});

test('a generated key round-trips with its password and refuses another', async t => {
  const pair = generateKeyPair('test-only password');
  const key = readSecretKey(pair.secretKey, 'test-only password');
  assert.equal(key.publicKeyBox, pair.publicKey);
  assert.throws(() => readSecretKey(pair.secretKey, ''), /Wrong password/);
  const path = await fixtureFile(t);
  assert.match(await verifyInstaller(path, await signFile(path, key, '2.0.0'), pair.publicKey), /\tversion:2\.0\.0$/);
});

test('signing refuses a missing version or one that would break the trusted comment', async t => {
  const path = await fixtureFile(t);
  const key = readSecretKey(tauri.privateKey, tauri.password);
  for (const version of ['', '1.0.0\tfile:x', '1.0.0\nx']) await assert.rejects(signFile(path, key, version));
});

test('the CLI reads the key from a path or from TAURI_SIGNING_PRIVATE_KEY like the Tauri CLI and prints no secrets', async t => {
  const dir = await folder(t);
  const keyPath = join(dir, 'disposable.key');
  const file = join(dir, 'setup.exe');
  await writeFile(file, 'installer bytes');
  const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.toUpperCase().startsWith('TAURI_')));
  const cli = (args, extra = {}) => run(process.execPath, [CLI, ...args], { env: { ...env, ...extra } });
  await cli(['generate', '--write-keys', keyPath, '--password', 'test-only']);
  await assert.rejects(cli(['generate', '--write-keys', keyPath]), /already exist/);
  const [secret, pub] = [await readFile(keyPath, 'utf8'), await readFile(`${keyPath}.pub`, 'utf8')];
  for (const extra of [{ TAURI_SIGNING_PRIVATE_KEY_PATH: keyPath }, { TAURI_SIGNING_PRIVATE_KEY: secret }]) {
    const { stdout, stderr } = await cli(['sign', '--app-version', '3.1.4', file], { ...extra, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'test-only' });
    const signature = await readFile(`${file}.sig`, 'utf8');
    assert.match(await verifyInstaller(file, signature, pub), /\tfile:setup\.exe\tversion:3\.1\.4$/);
    for (const output of [stdout, stderr]) for (const secretText of [secret, 'test-only', signature]) assert.ok(!output.includes(secretText));
  }
  const failure = await cli(['sign', '--app-version', '3.1.4', file], { TAURI_SIGNING_PRIVATE_KEY: secret, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'wrong' }).catch(error => error);
  assert.equal(failure.code, 1);
  assert.equal(failure.stderr.trim(), 'release-signer: Wrong password for the private key');
  await assert.rejects(cli(['sign', file], { TAURI_SIGNING_PRIVATE_KEY_PATH: keyPath }), /--app-version is required/);
  await assert.rejects(cli(['sign', '--app-version', '1.0.0', file], { TAURI_SIGNING_PRIVATE_KEY_PATH: keyPath, TAURI_SIGNING_PRIVATE_KEY: secret }), /not both/);
});
