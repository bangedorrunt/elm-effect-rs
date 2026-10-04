//! `Crypto.ts` port: randomness and digests behind a service.
//!
//! [`Crypto`] serves random bytes and digests so tests are deterministic:
//! [`TestCrypto`] replays a counter stream, [`SystemCrypto`] reads the OS.
//! [`random_between`]/[`random_int_between`] honor half-open ranges;
//! [`uuid_v4`]/[`uuid_v7`]/[`ulid`] format caller-supplied bytes plus a
//! timestamp, so formatting is pure and separately testable. Digest
//! algorithms are the spec literals (`SHA-256` implemented; the rest
//! report [`CryptoError::Unsupported`] instead of pretending).

use std::sync::{Arc, Mutex};

/// Digest algorithm literals from the spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestAlgorithm {
    /// SHA-1 (legacy only).
    Sha1,
    /// SHA-256.
    Sha256,
    /// SHA-384.
    Sha384,
    /// SHA-512.
    Sha512,
}

impl DigestAlgorithm {
    /// Parse the spec literal.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "SHA-1" => Some(Self::Sha1),
            "SHA-256" => Some(Self::Sha256),
            "SHA-384" => Some(Self::Sha384),
            "SHA-512" => Some(Self::Sha512),
            _ => None,
        }
    }

    /// Spec literal.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::Sha384 => "SHA-384",
            Self::Sha512 => "SHA-512",
        }
    }
}

/// Crypto failure.
#[derive(Clone, Debug, PartialEq)]
pub enum CryptoError {
    /// No OS randomness available.
    Unavailable(String),
    /// Algorithm not implemented here.
    Unsupported(String),
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(m) => write!(f, "randomness unavailable: {m}"),
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for CryptoError {}

/// Service type ID (`Crypto.Crypto` upstream).
pub const TYPE_ID: &str = "elm-effect/Crypto";

/// Randomness + digest service.
pub trait Crypto: Send + Sync {
    /// Fill with random bytes.
    fn random_bytes(&self, n: usize) -> Result<Vec<u8>, CryptoError>;
    /// Digest data (default: SHA-256).
    fn digest(&self, algorithm: DigestAlgorithm, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let _ = algorithm;
        Ok(sha256(data).to_vec())
    }
}

/// Deterministic replay service for tests: byte counter stream.
#[derive(Clone)]
pub struct TestCrypto {
    next: Arc<Mutex<u64>>,
}

impl TestCrypto {
    /// Start the counter at zero.
    pub fn new() -> Self {
        Self { next: Arc::new(Mutex::new(0)) }
    }

    /// Current counter (test introspection).
    pub fn counter(&self) -> u64 {
        *self.next.lock().unwrap()
    }
}

impl Default for TestCrypto {
    fn default() -> Self {
        Self::new()
    }
}

impl Crypto for TestCrypto {
    fn random_bytes(&self, n: usize) -> Result<Vec<u8>, CryptoError> {
        let mut guard = self.next.lock().unwrap();
        let out: Vec<u8> = (0..n).map(|i| ((*guard + i as u64) % 256) as u8).collect();
        *guard += n as u64;
        Ok(out)
    }

    fn digest(&self, _algorithm: DigestAlgorithm, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        Ok(data.to_vec())
    }
}

/// OS randomness service (`getrandom` substitute).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemCrypto;

impl Crypto for SystemCrypto {
    fn random_bytes(&self, n: usize) -> Result<Vec<u8>, CryptoError> {
        #[cfg(unix)]
        {
            use std::io::Read;
            let mut f =
                std::fs::File::open("/dev/urandom").map_err(|e| CryptoError::Unavailable(e.to_string()))?;
            let mut out = vec![0u8; n];
            f.read_exact(&mut out).map_err(|e| CryptoError::Unavailable(e.to_string()))?;
            Ok(out)
        }
        #[cfg(not(unix))]
        {
            let _ = n;
            Err(CryptoError::Unavailable("no OS randomness on this platform".to_string()))
        }
    }

    fn digest(&self, algorithm: DigestAlgorithm, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        match algorithm {
            DigestAlgorithm::Sha256 => Ok(sha256(data).to_vec()),
            other => Err(CryptoError::Unsupported(format!("{}: only SHA-256 here", other.name()))),
        }
    }
}

/// Random float in `[min, max)`: adjacent inputs map adjacently.
pub fn random_between(crypto: &impl Crypto, min: f64, max: f64) -> Result<f64, CryptoError> {
    let bytes = crypto.random_bytes(8)?;
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes);
    let unit = u64::from_le_bytes(raw) as f64 / u64::MAX as f64;
    Ok(min + unit * (max - min))
}

/// Random integer in `[min, max)` (upper bound excluded).
pub fn random_int_between(crypto: &impl Crypto, min: i64, max: i64) -> Result<i64, CryptoError> {
    if min >= max {
        return Err(CryptoError::Unavailable("empty range".to_string()));
    }
    let span = (max - min) as u64;
    let bytes = crypto.random_bytes(8)?;
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes);
    Ok(min + (u64::from_le_bytes(raw) % span) as i64)
}

/// UUID v4 text from 16 random bytes.
pub fn uuid_v4(random: &[u8; 16]) -> String {
    let mut b = *random;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15]
    )
}

/// UUID v7 text from a millisecond timestamp plus 10 random bytes.
pub fn uuid_v7(millis: u64, random: &[u8; 10]) -> String {
    let time_hex = format!("{millis:012x}");
    let rand_hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-7{}-{}-{}",
        &time_hex[..8],
        &time_hex[8..12],
        &rand_hex[..3],
        &rand_hex[3..7],
        &rand_hex[7..12]
    )
}

/// ULID text from a millisecond timestamp plus 10 random bytes; an
/// overflowing timestamp clamps instead of wrapping.
pub fn ulid(millis: u64, random: &[u8; 10]) -> String {
    const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let millis = millis.min((1 << 48) - 1);
    let mut bits: u128 = (millis as u128) << 80;
    for (i, b) in random.iter().enumerate() {
        bits |= (*b as u128) << (8 * (9 - i));
    }
    let mut out = String::with_capacity(26);
    for i in (0..26).rev() {
        out.push(CROCKFORD[((bits >> (5 * i)) & 31) as usize] as char);
    }
    out
}

/// SHA-256, hand-rolled so the crate stays dependency-free.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}
