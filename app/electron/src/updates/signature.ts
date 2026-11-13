import { createHash, createPublicKey, verify } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { readFile } from 'node:fs/promises';

function base64(value: string): Buffer {
  const s = value.trim();
  if (!s || s.length > 16384 || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(s)) throw new Error('Invalid base64');
  return Buffer.from(s, 'base64');
}

/** Tauri minisign-verify: Ed (legacy) or ED (BLAKE2b-512), plus global signature. */
export async function verifyInstaller(path: string, wrappedSignature: string, wrappedKey: string): Promise<string> {
  return (await verifyFile(path, wrappedSignature, wrappedKey)).comment;
}

/**
 * Like verifyInstaller, and also returns the SHA-256 of the bytes the signature check read, from the same pass over
 * the file. The bridge hands that hash to bridge.ps1, which refuses an MSI with any other hash.
 */
export async function verifyFile(path: string, wrappedSignature: string, wrappedKey: string): Promise<{ comment: string; sha256: string }> {
  const keyLines = base64(wrappedKey).toString('utf8').split(/\r?\n/);
  const lines = base64(wrappedSignature).toString('utf8').split(/\r?\n/);
  const key = base64(keyLines[1] ?? '');
  const packet = base64(lines[1] ?? '');
  const global = base64(lines[3] ?? '');
  if (key.length !== 42 || packet.length !== 74 || global.length !== 64 ||
      !['Ed', 'ED'].includes(key.subarray(0, 2).toString()) ||
      !['Ed', 'ED'].includes(packet.subarray(0, 2).toString()) ||
      !key.subarray(2, 10).equals(packet.subarray(2, 10)) ||
      !lines[2]?.startsWith('trusted comment: ')) throw new Error('Invalid signature');
  const publicKey = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key.subarray(10)]), format: 'der', type: 'spki' });
  const signature = packet.subarray(10);
  const comment = lines[2].slice(17);
  if (!verify(null, Buffer.concat([signature, Buffer.from(comment)]), publicKey, global)) throw new Error('Invalid comment signature');
  let data: Uint8Array;
  const sha256 = createHash('sha256');
  if (packet[1] === 0x44) {
    const { blake2b } = await import('@noble/hashes/blake2.js');
    const hash = blake2b.create({ dkLen: 64 });
    for await (const chunk of createReadStream(path)) { hash.update(chunk); sha256.update(chunk); }
    data = hash.digest();
  } else {
    // Legacy Ed25519 needs the whole message; the downloader caps the file at 256 MiB.
    data = await readFile(path);
    sha256.update(data);
  }
  if (!verify(null, data, publicKey, signature)) throw new Error('Invalid installer signature');
  return { comment, sha256: sha256.digest('hex') };
}

export function compareVersions(a: string, b: string): number {
  const parse = (s: string) => {
    const m = /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(s);
    if (!m || m[4]?.split('.').some(x => /^\d+$/.test(x) && x.length > 1 && x[0] === '0')) throw new Error('Invalid version');
    return m;
  };
  const x = parse(a), y = parse(b);
  const cmp = (l: bigint | string, r: bigint | string) => l < r ? -1 : l > r ? 1 : 0;
  for (let i = 1; i <= 3; i++) { const n = cmp(BigInt(x[i]), BigInt(y[i])); if (n) return n; }
  if (!x[4] || !y[4]) return x[4] ? -1 : y[4] ? 1 : 0;
  const xp = x[4].split('.'), yp = y[4].split('.');
  for (let i = 0; i < Math.max(xp.length, yp.length); i++) {
    if (xp[i] === undefined) return -1;
    if (yp[i] === undefined) return 1;
    const xn = /^\d+$/.test(xp[i]), yn = /^\d+$/.test(yp[i]);
    const n = xn && yn ? cmp(BigInt(xp[i]), BigInt(yp[i])) : xn !== yn ? (xn ? -1 : 1) : cmp(xp[i], yp[i]);
    if (n) return n;
  }
  return 0;
}
