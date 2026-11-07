//! net-v2 items and cryptography, exactly as `schema/protocol/net-v2.md` states them: handshake
//! items, SRP-6a over RFC 5054's 3072-bit group with SHA-512, the pairing transcript and proofs,
//! the X25519 session keys, and AES-256-GCM records. Both the Companion's client and the fake
//! Device in tests build on these, and `net-v2-vectors.json` checks them.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use num_bigint::BigUint;
use sha2::{Digest, Sha256, Sha512};
use srp::client::SrpClient;
use srp::groups::G_3072;
use subtle::ConstantTimeEq;
use td_core::Frame;
use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret};
use zeroize::Zeroize;

use super::{NetError, Refusal};

type HmacSha256 = Hmac<Sha256>;

pub const VERSION: u8 = 2;
pub const DEVICE_ID_LEN: usize = 6;
pub const COMPANION_ID_LEN: usize = 16;
pub const WINDOW_ID_LEN: usize = 16;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 32;
pub const PAD_LEN: usize = 384;
pub const LTK_LEN: usize = 32;
pub const SHARE_LEN: usize = 32;
pub const WRAPPED_KEY_LEN: usize = 48;
pub const TAG_LEN: usize = 16;
pub const MIN_HANDSHAKE_BODY: usize = 3;
pub const MAX_HANDSHAKE_BODY: usize = 512;
pub const MIN_RECORD_BODY: usize = 24;
pub const MAX_RECORD_BODY: usize = 1_048;
/// Records per direction per key. A sender never seals counter `2^24` and a receiver closes
/// rather than accept it; the session ends there and never rekeys.
pub const COUNTER_LIMIT: u64 = 1 << 24;
/// A sender ends the session between requests once fewer records than this remain.
pub const RECORD_RESERVE: u64 = 1024;
pub const PAIR_WINDOW_MS: u32 = 120_000;
pub const NAME_MAX: usize = 64;

pub const PAIR_REQUEST: u8 = 0x10;
pub const PAIR_OFFER: u8 = 0x11;
pub const PAIR_START: u8 = 0x12;
pub const PAIR_CHALLENGE: u8 = 0x13;
pub const PAIR_PROOF: u8 = 0x14;
pub const PAIR_ACCEPT: u8 = 0x15;
pub const PAIR_COMMIT: u8 = 0x16;
pub const PAIR_STORED: u8 = 0x17;
pub const SESSION_OPEN: u8 = 0x20;
pub const SESSION_CHALLENGE: u8 = 0x21;
pub const SESSION_PROOF: u8 = 0x22;
pub const SESSION_READY: u8 = 0x23;
pub const ERROR: u8 = 0x7f;

/// Direction 0 is Companion to Device, 1 is Device to Companion.
pub const C2D: u8 = 0;
pub const D2C: u8 = 1;

fn protocol(detail: impl Into<String>) -> NetError {
    NetError::Protocol(detail.into())
}

fn auth(detail: impl Into<String>) -> NetError {
    NetError::Auth(detail.into())
}

/// `\p{Cf}`, the format characters, pinned to Unicode 17.0 as net-v2 lists them.
pub const FORMAT_RANGES: &[(u32, u32)] = &[
    (0x00ad, 0x00ad),
    (0x0600, 0x0605),
    (0x061c, 0x061c),
    (0x06dd, 0x06dd),
    (0x070f, 0x070f),
    (0x0890, 0x0891),
    (0x08e2, 0x08e2),
    (0x180e, 0x180e),
    (0x200b, 0x200f),
    (0x202a, 0x202e),
    (0x2060, 0x2064),
    (0x2066, 0x206f),
    (0xfeff, 0xfeff),
    (0xfff9, 0xfffb),
    (0x110bd, 0x110bd),
    (0x110cd, 0x110cd),
    (0x13430, 0x1343f),
    (0x1bca0, 0x1bca3),
    (0x1d173, 0x1d17a),
    (0xe0001, 0xe0001),
    (0xe0020, 0xe007f),
];

/// Letters that render as blank space.
pub const HANGUL_FILLERS: [char; 4] = ['\u{115f}', '\u{1160}', '\u{3164}', '\u{ffa0}'];

/// Whether `c` may appear in a Companion name: not a control character (`\p{Cc}`), a
/// format character (`\p{Cf}`) or a Hangul filler.
pub fn name_char_allowed(c: char) -> bool {
    let u = u32::from(c);
    !c.is_control()
        && !FORMAT_RANGES.iter().any(|&(lo, hi)| (lo..=hi).contains(&u))
        && !HANGUL_FILLERS.contains(&c)
}

/// Whether a Companion name may go in `pair.request`: 1 to 64 UTF-8 bytes of allowed
/// characters.
pub fn valid_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len()) && name.chars().all(name_char_allowed)
}

/// Whether a code is the six ASCII digits the Device shows.
pub fn valid_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())
}

fn check_fields(kind: u8, fields: &[u8]) -> Result<(), NetError> {
    if kind == PAIR_REQUEST {
        if !(18..=17 + NAME_MAX).contains(&fields.len())
            || usize::from(fields[16]) != fields.len() - 17
        {
            return Err(protocol("invalid pair.request"));
        }
        let name = std::str::from_utf8(&fields[17..]).map_err(|_| protocol("name is not UTF-8"))?;
        if !valid_name(name) {
            return Err(protocol("invalid Companion name"));
        }
        return Ok(());
    }
    let size = match kind {
        PAIR_OFFER => 26,
        PAIR_START => 416,
        PAIR_CHALLENGE => 432,
        PAIR_PROOF | PAIR_COMMIT | PAIR_STORED | SESSION_PROOF | SESSION_READY => 32,
        PAIR_ACCEPT => 80,
        SESSION_OPEN => 54,
        SESSION_CHALLENGE => 64,
        ERROR => 1,
        _ => return Err(protocol(format!("unknown handshake kind 0x{kind:02x}"))),
    };
    if fields.len() != size {
        return Err(protocol(format!(
            "invalid length for handshake kind 0x{kind:02x}"
        )));
    }
    if kind == ERROR && Refusal::from_u8(fields[0]).is_none() {
        return Err(protocol("unknown error reason"));
    }
    if kind == PAIR_OFFER {
        let left = u32::from_be_bytes([fields[22], fields[23], fields[24], fields[25]]);
        if left == 0 || left > PAIR_WINDOW_MS {
            return Err(protocol("invalid window duration"));
        }
    }
    Ok(())
}

/// A complete handshake item, length prefix included.
pub fn handshake(kind: u8, fields: &[u8]) -> Result<Vec<u8>, NetError> {
    check_fields(kind, fields)?;
    let mut item = Vec::with_capacity(fields.len() + 4);
    item.extend_from_slice(&((fields.len() + 2) as u16).to_be_bytes());
    item.extend_from_slice(&[kind, VERSION]);
    item.extend_from_slice(fields);
    Ok(item)
}

/// The handshake error item for `reason`.
pub fn error_item(reason: Refusal) -> Vec<u8> {
    handshake(ERROR, &[reason as u8]).expect("every reason encodes")
}

/// Splits a complete handshake item into its kind and fields. A version other than 2 is
/// [`NetError::Version`], which a Device answers with `version`.
pub fn parse_handshake(item: &[u8]) -> Result<(u8, &[u8]), NetError> {
    if item.len() < 2 + MIN_HANDSHAKE_BODY || item.len() > 2 + MAX_HANDSHAKE_BODY {
        return Err(protocol("invalid handshake size"));
    }
    if usize::from(u16::from_be_bytes([item[0], item[1]])) != item.len() - 2 {
        return Err(protocol("invalid handshake prefix"));
    }
    if item[3] != VERSION {
        return Err(NetError::Version(item[3]));
    }
    let (kind, fields) = (item[2], &item[4..]);
    check_fields(kind, fields)?;
    Ok((kind, fields))
}

/// What an item's length prefix may say: handshake items before `session.ready`, records
/// after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Handshake,
    Record,
}

impl Phase {
    /// The body length a prefix announces, checked against this phase's bounds before
    /// anything is read or kept.
    pub fn body_len(self, prefix: [u8; 2]) -> Result<usize, NetError> {
        let len = usize::from(u16::from_be_bytes(prefix));
        let (min, max) = match self {
            Phase::Handshake => (MIN_HANDSHAKE_BODY, MAX_HANDSHAKE_BODY),
            Phase::Record => (MIN_RECORD_BODY, MAX_RECORD_BODY),
        };
        if (min..=max).contains(&len) {
            Ok(len)
        } else {
            Err(protocol(format!("item length {len} is out of bounds")))
        }
    }
}

/// net-v2's share check, step 1: bit 255 clear and `u < p`, `p = 2^255 - 19`.
pub fn canonical_share(share: &[u8; SHARE_LEN]) -> bool {
    if share[31] & 0x80 != 0 {
        return false;
    }
    // u >= p only when the top byte is 0x7f, the middle bytes are all 0xff and the low
    // byte is at least 0xed.
    !(share[31] == 0x7f && share[1..31].iter().all(|&b| b == 0xff) && share[0] >= 0xed)
}

/// `Z` from our secret and the peer's share, under net-v2's share checks: a non-canonical
/// share, and a share that gives an all-zero `Z`, fail the same way, before any key is
/// derived.
pub fn agree(secret: EphemeralSecret, share: &[u8; SHARE_LEN]) -> Result<SharedSecret, NetError> {
    if !canonical_share(share) {
        return Err(auth("invalid X25519 share"));
    }
    let shared = secret.diffie_hellman(&PublicKey::from(*share));
    if !shared.was_contributory() {
        return Err(auth("invalid X25519 share"));
    }
    Ok(shared)
}

/// Constant-time equality for proofs and tags.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part);
    }
    hash.finalize().into()
}

pub fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key).expect("HMAC takes any key");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

/// HKDF-Extract with `salt`, then `HKDF-Expand(PRK, "touchdeck/net-v2/" || label, 32)` per label.
fn extract_and_derive<const N: usize>(
    salt: &[u8; 32],
    ikm: &[u8],
    labels: [&str; N],
) -> ([u8; 32], [[u8; 32]; N]) {
    let (mut extracted, hk) = Hkdf::<Sha256>::extract(Some(salt), ikm);
    let mut keys = [[0u8; 32]; N];
    for (key, label) in keys.iter_mut().zip(labels) {
        hk.expand(format!("touchdeck/net-v2/{label}").as_bytes(), key)
            .expect("HKDF expands 32 bytes");
    }
    let mut prk = [0u8; 32];
    prk.copy_from_slice(&extracted);
    extracted.as_mut_slice().zeroize();
    (prk, keys)
}

// --- SRP-6a ---------------------------------------------------------------------------------

/// `PAD(n)`: the 384-byte big-endian encoding.
pub fn pad(n: &BigUint) -> [u8; PAD_LEN] {
    let raw = n.to_bytes_be();
    let mut out = [0u8; PAD_LEN];
    out[PAD_LEN - raw.len()..].copy_from_slice(&raw);
    out
}

/// Overwrites a secret number in place before it is freed. num-bigint 0.4 has no `zeroize`,
/// so this is best effort: it wipes the named values, not the temporaries inside `modpow`.
fn wipe(n: &mut BigUint) {
    let digits = (n.bits() as usize).div_ceil(32);
    n.assign_from_slice(&vec![0u32; digits]);
}

/// `MIN(n)`: the shortest nonempty big-endian encoding.
pub fn min(n: &BigUint) -> Vec<u8> {
    n.to_bytes_be()
}

/// `I = "touchdeck:" || hex(device_id) || ":" || hex(companion_id)`.
pub fn identity(device_id: &[u8; DEVICE_ID_LEN], companion_id: &[u8; COMPANION_ID_LEN]) -> Vec<u8> {
    format!("touchdeck:{}:{}", hex(device_id), hex(companion_id)).into_bytes()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A nonzero 32-byte exponent, or `None` for all zeros.
fn exponent(bytes: &[u8; 32]) -> Option<BigUint> {
    let n = BigUint::from_bytes_be(bytes);
    (n != BigUint::ZERO).then_some(n)
}

/// `0 < value < N`.
pub fn public_value(bytes: &[u8]) -> Result<BigUint, NetError> {
    let value = BigUint::from_bytes_be(bytes);
    if bytes.len() != PAD_LEN || value == BigUint::ZERO || value >= G_3072.n {
        return Err(protocol("invalid SRP public value"));
    }
    Ok(value)
}

/// `k = INT(H512(PAD(N) || PAD(g)))`.
pub fn srp_k() -> BigUint {
    let mut hash = Sha512::new();
    hash.update(pad(&G_3072.n));
    hash.update(pad(&G_3072.g));
    BigUint::from_bytes_be(&hash.finalize())
}

/// `u = INT(H512(PAD(A) || PAD(B)))`, which must not be zero.
pub fn srp_u(a_pub: &[u8; PAD_LEN], b_pub: &[u8; PAD_LEN]) -> Result<BigUint, NetError> {
    let mut hash = Sha512::new();
    hash.update(a_pub);
    hash.update(b_pub);
    let u = BigUint::from_bytes_be(&hash.finalize());
    if u == BigUint::ZERO {
        return Err(auth("zero SRP scrambler"));
    }
    Ok(u)
}

/// `x = INT(H512(salt || H512(I || ":" || P)))`.
pub fn srp_x(identity: &[u8], code: &str, salt: &[u8; SALT_LEN]) -> BigUint {
    let hash = SrpClient::<Sha512>::compute_identity_hash(identity, code.as_bytes());
    SrpClient::<Sha512>::compute_x(hash.as_slice(), salt)
}

/// `A = g^a mod N`, padded.
pub fn srp_client_public(a: &[u8; 32]) -> Result<[u8; PAD_LEN], NetError> {
    let a = exponent(a).ok_or_else(|| protocol("zero SRP exponent"))?;
    let a_pub = pad(&G_3072.g.modpow(&a, &G_3072.n));
    public_value(&a_pub)?;
    Ok(a_pub)
}

/// `K_srp = H512(MIN(S))` for an `S` that isn't zero.
fn srp_key(mut s: BigUint) -> Result<[u8; 64], NetError> {
    if s == BigUint::ZERO {
        return Err(auth("zero SRP secret"));
    }
    let mut bytes = min(&s);
    wipe(&mut s);
    let key = Sha512::digest(&bytes).into();
    bytes.zeroize();
    Ok(key)
}

/// The Companion's side: `K_srp` from `S_companion = (B - k*g^x mod N)^(a + u*x) mod N`.
pub fn srp_client_key(
    identity: &[u8],
    code: &str,
    salt: &[u8; SALT_LEN],
    a: &[u8; 32],
    b_pub: &[u8; PAD_LEN],
) -> Result<[u8; 64], NetError> {
    let a_pub = srp_client_public(a)?;
    let mut a = exponent(a).ok_or_else(|| protocol("zero SRP exponent"))?;
    let b = public_value(b_pub)?;
    let u = srp_u(&a_pub, b_pub)?;
    let (n, g) = (&G_3072.n, &G_3072.g);
    let mut x = srp_x(identity, code, salt);
    let k = srp_k();
    let mut kgx = (&k * g.modpow(&x, n)) % n;
    let zero_base = (&b + n - &kgx) % n == BigUint::ZERO;
    wipe(&mut kgx);
    let s = (!zero_base)
        .then(|| SrpClient::<Sha512>::new(&G_3072).compute_premaster_secret(&b, &k, &x, &a, &u));
    wipe(&mut x);
    wipe(&mut a);
    match s {
        Some(s) => srp_key(s),
        None => Err(auth("zero SRP client base")),
    }
}

/// The Device's side of one attempt: its public `B`, kept with what it needs for `S`.
pub struct SrpServer {
    pub b_pub: [u8; PAD_LEN],
    b: BigUint,
    v: BigUint,
}

impl SrpServer {
    /// `B = (k*v + g^b) mod N` with `v = g^x mod N`.
    pub fn new(
        identity: &[u8],
        code: &str,
        salt: &[u8; SALT_LEN],
        b: &[u8; 32],
    ) -> Result<Self, NetError> {
        let b = exponent(b).ok_or_else(|| protocol("zero SRP exponent"))?;
        let (n, g) = (&G_3072.n, &G_3072.g);
        let mut x = srp_x(identity, code, salt);
        let v = g.modpow(&x, n);
        wipe(&mut x);
        let b_pub = pad(&((srp_k() * &v + g.modpow(&b, n)) % n));
        public_value(&b_pub)?;
        Ok(SrpServer { b_pub, b, v })
    }

    /// `K_srp` from `S_device = (A * v^u mod N)^b mod N`.
    pub fn key(&self, a_pub: &[u8; PAD_LEN]) -> Result<[u8; 64], NetError> {
        let a = public_value(a_pub)?;
        let u = srp_u(a_pub, &self.b_pub)?;
        let n = &G_3072.n;
        let s = ((a * self.v.modpow(&u, n)) % n).modpow(&self.b, n);
        srp_key(s)
    }
}

impl Drop for SrpServer {
    fn drop(&mut self) {
        wipe(&mut self.b);
        wipe(&mut self.v);
    }
}

// --- Pairing transcript, proofs and key delivery ---------------------------------------------

/// The pairing keys and proofs for one attempt.
pub struct PairKeys {
    pub transcript: [u8; 32],
    pub prk: [u8; 32],
    pub client: [u8; 32],
    pub deck: [u8; 32],
    pub wrap: [u8; 32],
    pub commit: [u8; 32],
    pub client_proof: [u8; 32],
    pub deck_proof: [u8; 32],
}

impl Drop for PairKeys {
    fn drop(&mut self) {
        self.prk.zeroize();
        self.client.zeroize();
        self.deck.zeroize();
        self.wrap.zeroize();
        self.commit.zeroize();
    }
}

impl PairKeys {
    /// Derives from `K_srp` and the four complete items `request`, `offer`, `start`, `challenge`.
    pub fn derive(
        k_srp: &[u8; 64],
        request: &[u8],
        offer: &[u8],
        start: &[u8],
        challenge: &[u8],
    ) -> Result<Self, NetError> {
        for (item, kind) in [
            (request, PAIR_REQUEST),
            (offer, PAIR_OFFER),
            (start, PAIR_START),
            (challenge, PAIR_CHALLENGE),
        ] {
            if parse_handshake(item)?.0 != kind {
                return Err(protocol("unexpected item in the pairing transcript"));
            }
        }
        let transcript = sha256(&[b"touchdeck/net-v2/pair\0", request, offer, start, challenge]);
        let (prk, [client, deck, wrap, commit]) = extract_and_derive(
            &transcript,
            k_srp,
            ["pair/client", "pair/deck", "pair/wrap", "pair/commit"],
        );
        let client_proof = hmac(&client, &[&transcript]);
        let deck_proof = hmac(&deck, &[&transcript, &client_proof]);
        Ok(PairKeys {
            transcript,
            prk,
            client,
            deck,
            wrap,
            commit,
            client_proof,
            deck_proof,
        })
    }

    fn wrap_aad(&self) -> [u8; 96] {
        let mut aad = [0u8; 96];
        aad[..32].copy_from_slice(&self.transcript);
        aad[32..64].copy_from_slice(&self.client_proof);
        aad[64..].copy_from_slice(&self.deck_proof);
        aad
    }

    /// `wrapped_key`: the LTK sealed under `kw` with a zero nonce.
    pub fn wrap_key(&self, ltk: &[u8; LTK_LEN]) -> [u8; WRAPPED_KEY_LEN] {
        let sealed = seal(&self.wrap, &[0u8; 12], ltk, &self.wrap_aad());
        sealed.try_into().expect("32 bytes and a tag")
    }

    /// Checks `device_proof` in `pair.accept` fields, then opens the LTK.
    pub fn accept(&self, fields: &[u8]) -> Result<[u8; LTK_LEN], NetError> {
        if fields.len() != 80 || !same(&fields[..32], &self.deck_proof) {
            return Err(auth("bad device proof"));
        }
        let mut ltk = open(&self.wrap, &[0u8; 12], &fields[32..], &self.wrap_aad())?;
        let key: [u8; LTK_LEN] = ltk
            .as_slice()
            .try_into()
            .map_err(|_| auth("bad wrapped key"))?;
        ltk.zeroize();
        Ok(key)
    }

    pub fn commit_proof(&self, ltk: &[u8; LTK_LEN]) -> [u8; 32] {
        hmac(
            &self.commit,
            &[b"commit", &self.transcript, &sha256(&[ltk])],
        )
    }

    pub fn stored_proof(&self, ltk: &[u8; LTK_LEN]) -> [u8; 32] {
        hmac(
            &self.commit,
            &[b"stored", &self.transcript, &sha256(&[ltk])],
        )
    }
}

// --- Session ---------------------------------------------------------------------------------

/// The session keys and proofs, from the LTK and the X25519 result `Z`.
pub struct SessionKeys {
    pub transcript: [u8; 32],
    pub prk: [u8; 32],
    pub deck: [u8; 32],
    pub client: [u8; 32],
    pub ready: [u8; 32],
    pub c2d: [u8; 32],
    pub d2c: [u8; 32],
    pub deck_proof: [u8; 32],
    pub client_proof: [u8; 32],
    pub ready_proof: [u8; 32],
}

impl Drop for SessionKeys {
    fn drop(&mut self) {
        self.prk.zeroize();
        self.deck.zeroize();
        self.client.zeroize();
        self.ready.zeroize();
        self.c2d.zeroize();
        self.d2c.zeroize();
    }
}

impl SessionKeys {
    /// Rejects an all-zero `Z`, which a low-order share produces.
    pub fn derive(
        ltk: &[u8; LTK_LEN],
        device_id: &[u8; DEVICE_ID_LEN],
        companion_id: &[u8; COMPANION_ID_LEN],
        client_share: &[u8; SHARE_LEN],
        device_share: &[u8; SHARE_LEN],
        z: &[u8; 32],
    ) -> Result<Self, NetError> {
        if same(z, &[0u8; 32]) {
            return Err(auth("non-contributory X25519 share"));
        }
        let transcript = sha256(&[
            b"touchdeck/net-v2/session\0",
            device_id,
            companion_id,
            client_share,
            device_share,
        ]);
        let mut ikm = [0u8; 64];
        ikm[..32].copy_from_slice(ltk);
        ikm[32..].copy_from_slice(z);
        let (prk, [deck, client, ready, c2d, d2c]) = extract_and_derive(
            &transcript,
            &ikm,
            [
                "session/deck",
                "session/client",
                "session/ready",
                "session/c2d",
                "session/d2c",
            ],
        );
        ikm.zeroize();
        let deck_proof = hmac(&deck, &[&transcript]);
        let client_proof = hmac(&client, &[&transcript, &deck_proof]);
        let ready_proof = hmac(&ready, &[&transcript, &client_proof]);
        Ok(SessionKeys {
            transcript,
            prk,
            deck,
            client,
            ready,
            c2d,
            d2c,
            deck_proof,
            client_proof,
            ready_proof,
        })
    }
}

// --- AES-256-GCM and records -----------------------------------------------------------------

fn seal(key: &[u8; 32], nonce: &[u8; 12], plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    Aes256Gcm::new_from_slice(key)
        .expect("a 32-byte key")
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("AES-GCM seals any record")
}

fn open(key: &[u8; 32], nonce: &[u8; 12], sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>, NetError> {
    Aes256Gcm::new_from_slice(key)
        .expect("a 32-byte key")
        .decrypt(Nonce::from_slice(nonce), Payload { msg: sealed, aad })
        .map_err(|_| auth("record tag failed"))
}

/// The nonce and AAD of record `counter` in `direction` with body length `len`.
pub fn record_parameters(
    direction: u8,
    counter: u64,
    len: usize,
) -> Result<([u8; 12], [u8; 15]), NetError> {
    if counter >= COUNTER_LIMIT {
        return Err(protocol("record counter exhausted"));
    }
    if !(MIN_RECORD_BODY..=MAX_RECORD_BODY).contains(&len) {
        return Err(protocol("invalid record length"));
    }
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    let mut aad = [0u8; 15];
    aad[..4].copy_from_slice(b"TDN2");
    aad[4] = direction;
    aad[5..13].copy_from_slice(&counter.to_be_bytes());
    aad[13..].copy_from_slice(&(len as u16).to_be_bytes());
    Ok((nonce, aad))
}

/// One direction's sealing half. Any failure closes it for good.
pub struct RecordSender {
    key: [u8; 32],
    direction: u8,
    counter: u64,
    closed: bool,
}

impl RecordSender {
    pub fn new(key: &[u8; 32], direction: u8) -> Self {
        RecordSender {
            key: *key,
            direction,
            counter: 0,
            closed: false,
        }
    }

    /// Starts at `counter`, for tests of the counter limit.
    pub fn at(key: &[u8; 32], direction: u8, counter: u64) -> Self {
        RecordSender {
            counter,
            ..Self::new(key, direction)
        }
    }

    /// The counter the next record takes.
    pub fn counter(&self) -> u64 {
        self.counter
    }

    /// Records this key can still seal.
    pub fn remaining(&self) -> u64 {
        COUNTER_LIMIT.saturating_sub(self.counter)
    }

    /// Moves the counter forward to `counter`, for tests of the limit. It never goes back, so
    /// no nonce repeats.
    #[doc(hidden)]
    pub fn skip_to(&mut self, counter: u64) {
        self.counter = self.counter.max(counter);
    }

    /// Seals one complete link frame body into an item.
    pub fn seal(&mut self, body: &[u8]) -> Result<Vec<u8>, NetError> {
        if self.closed {
            return Err(protocol("record sender closed"));
        }
        let sealed = (|| {
            Frame::decode_body(body).map_err(|e| protocol(format!("invalid link frame: {e}")))?;
            let len = body.len() + TAG_LEN;
            let (nonce, aad) = record_parameters(self.direction, self.counter, len)?;
            let mut item = Vec::with_capacity(len + 2);
            item.extend_from_slice(&(len as u16).to_be_bytes());
            item.extend_from_slice(&seal(&self.key, &nonce, body, &aad));
            Ok(item)
        })();
        match sealed {
            Ok(item) => {
                self.counter += 1;
                Ok(item)
            }
            Err(error) => {
                self.close();
                Err(error)
            }
        }
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.key.zeroize();
    }
}

impl Drop for RecordSender {
    fn drop(&mut self) {
        self.close();
    }
}

/// One direction's opening half. Only the next counter opens; anything else closes it.
pub struct RecordReceiver {
    key: [u8; 32],
    direction: u8,
    counter: u64,
    closed: bool,
}

impl RecordReceiver {
    pub fn new(key: &[u8; 32], direction: u8) -> Self {
        RecordReceiver {
            key: *key,
            direction,
            counter: 0,
            closed: false,
        }
    }

    pub fn at(key: &[u8; 32], direction: u8, counter: u64) -> Self {
        RecordReceiver {
            counter,
            ..Self::new(key, direction)
        }
    }

    /// The counter the next record must carry.
    pub fn counter(&self) -> u64 {
        self.counter
    }

    /// Records this key can still open.
    pub fn remaining(&self) -> u64 {
        COUNTER_LIMIT.saturating_sub(self.counter)
    }

    /// Moves the counter forward to `counter`, for tests of the limit.
    #[doc(hidden)]
    pub fn skip_to(&mut self, counter: u64) {
        self.counter = self.counter.max(counter);
    }

    /// Authenticates a complete record item, then checks the link frame body inside it.
    pub fn open(&mut self, item: &[u8]) -> Result<Vec<u8>, NetError> {
        if self.closed {
            return Err(protocol("record receiver closed"));
        }
        let opened = (|| {
            if item.len() < 2 + MIN_RECORD_BODY || item.len() > 2 + MAX_RECORD_BODY {
                return Err(protocol("invalid record size"));
            }
            let len = usize::from(u16::from_be_bytes([item[0], item[1]]));
            if len != item.len() - 2 {
                return Err(protocol("invalid record prefix"));
            }
            let (nonce, aad) = record_parameters(self.direction, self.counter, len)?;
            let body = open(&self.key, &nonce, &item[2..], &aad)?;
            Frame::decode_body(&body).map_err(|e| protocol(format!("invalid link frame: {e}")))?;
            Ok(body)
        })();
        match opened {
            Ok(body) => {
                self.counter += 1;
                Ok(body)
            }
            Err(error) => {
                self.close();
                Err(error)
            }
        }
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.key.zeroize();
    }
}

impl Drop for RecordReceiver {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use td_core::Role;

    fn body(seq: u8) -> Vec<u8> {
        let frame = Frame::message(Role::Request(seq), b"{\"m\":\"ping\"}".to_vec());
        frame.encode_body().unwrap()
    }

    #[test]
    fn records_refuse_replay_reorder_and_a_closed_direction() {
        let key = [0x11u8; 32];
        let mut sender = RecordSender::new(&key, C2D);
        let first = sender.seal(&body(1)).unwrap();
        let second = sender.seal(&body(2)).unwrap();

        let mut reorder = RecordReceiver::new(&key, C2D);
        assert!(reorder.open(&second).is_err());
        assert!(
            reorder.open(&first).is_err(),
            "a failure closes the direction"
        );

        let mut replay = RecordReceiver::new(&key, C2D);
        assert_eq!(replay.open(&first).unwrap(), body(1));
        assert!(replay.open(&first).is_err());

        let mut wrong_direction = RecordReceiver::new(&key, D2C);
        assert!(
            wrong_direction.open(&first).is_err(),
            "reflection fails the AAD"
        );
    }

    #[test]
    fn truncated_and_overlong_records_close_the_receiver() {
        let key = [0x22u8; 32];
        let item = RecordSender::new(&key, D2C).seal(&body(3)).unwrap();
        assert!(
            RecordReceiver::new(&key, D2C)
                .open(&item[..item.len() - 1])
                .is_err()
        );
        let mut long = item.clone();
        long.push(0);
        assert!(RecordReceiver::new(&key, D2C).open(&long).is_err());
    }

    #[test]
    fn counters_stop_before_two_to_the_twenty_four() {
        let key = [0x33u8; 32];
        let mut last = RecordSender::at(&key, C2D, COUNTER_LIMIT - 1);
        let item = last.seal(&body(4)).unwrap();
        assert_eq!(
            RecordReceiver::at(&key, C2D, COUNTER_LIMIT - 1)
                .open(&item)
                .unwrap(),
            body(4)
        );
        assert!(last.seal(&body(5)).is_err(), "counter 2^24 never goes out");
        assert!(
            RecordReceiver::at(&key, C2D, COUNTER_LIMIT)
                .open(&item)
                .is_err()
        );
    }

    #[test]
    fn a_maximum_size_record_fits_and_one_byte_more_does_not() {
        let key = [0x44u8; 32];
        let full = Frame::message(Role::Event, vec![b'x'; td_core::frame::PAYLOAD_MAX])
            .encode_body()
            .unwrap();
        let item = RecordSender::new(&key, D2C).seal(&full).unwrap();
        assert_eq!(item.len(), 1_050);
        assert_eq!(RecordReceiver::new(&key, D2C).open(&item).unwrap(), full);
    }

    #[test]
    fn handshake_items_check_version_kind_and_length() {
        let busy = error_item(Refusal::Busy);
        assert_eq!(hex(&busy), "00037f0201");
        assert_eq!(parse_handshake(&busy).unwrap(), (ERROR, &[1u8][..]));
        let mut v1 = busy.clone();
        v1[3] = 1;
        assert!(matches!(parse_handshake(&v1), Err(NetError::Version(1))));
        assert!(
            parse_handshake(&[0, 3, 0x7f, 2, 10]).is_err(),
            "unknown reason"
        );
        assert!(
            parse_handshake(&[0, 4, 0x7f, 2, 1, 0]).is_err(),
            "trailing bytes"
        );
        assert!(
            parse_handshake(&[0, 3, 0x55, 2, 1]).is_err(),
            "unknown kind"
        );
        assert!(handshake(SESSION_PROOF, &[0; 31]).is_err());
    }

    #[test]
    fn names_refuse_controls_format_characters_and_fillers() {
        assert!(valid_name("Desk PC"));
        assert!(valid_name("Desk \u{1f5a5}\u{fe0f}"));
        assert!(valid_name(&"\u{e9}".repeat(32)));
        assert!(
            !valid_name(&"\u{e9}".repeat(33)),
            "64 bytes, not 64 characters"
        );
        assert!(!valid_name(""));
        assert!(!valid_name(&"a".repeat(65)));
        assert!(!valid_name("evil\u{202e}cp"));
        assert!(!valid_name("tab\there"));
        assert!(!valid_name("iso\u{2067}late"));
        assert!(!valid_name("zero\u{200b}width"));
        assert!(!valid_name("\u{feff}bom"));
        assert!(!valid_name("next\u{85}line"));
        assert!(!valid_name("blank\u{3164}"));
        assert!(valid_name("unassigned\u{2065}"));
        assert!(valid_code("012345"));
        assert!(!valid_code("12345"));
        assert!(!valid_code("12345a"));
    }

    #[test]
    fn length_prefixes_are_checked_per_phase() {
        assert_eq!(Phase::Handshake.body_len([0, 3]).unwrap(), 3);
        assert_eq!(Phase::Handshake.body_len([2, 0]).unwrap(), 512);
        assert!(Phase::Handshake.body_len([0, 2]).is_err());
        assert!(Phase::Handshake.body_len([2, 1]).is_err());
        assert_eq!(Phase::Record.body_len([0, 24]).unwrap(), 24);
        assert_eq!(Phase::Record.body_len([4, 24]).unwrap(), 1_048);
        assert!(Phase::Record.body_len([0, 23]).is_err());
        assert!(Phase::Record.body_len([4, 25]).is_err());
    }

    #[test]
    fn shares_at_or_above_p_or_with_bit_255_are_not_canonical() {
        let mut p = [0xffu8; 32];
        p[0] = 0xed;
        p[31] = 0x7f;
        assert!(!canonical_share(&p));
        p[0] = 0xec;
        assert!(
            canonical_share(&p),
            "p - 1 is canonical, though small order"
        );
        let mut high = [0u8; 32];
        high[0] = 9;
        assert!(canonical_share(&high));
        high[31] = 0x80;
        assert!(!canonical_share(&high));
    }

    #[test]
    fn srp_public_values_must_lie_between_zero_and_n() {
        assert!(public_value(&[0u8; PAD_LEN]).is_err());
        assert!(public_value(&pad(&G_3072.n)).is_err());
        assert!(public_value(&pad(&BigUint::from(1u8))).is_ok());
        assert!(srp_client_public(&[0u8; 32]).is_err());
    }
}
