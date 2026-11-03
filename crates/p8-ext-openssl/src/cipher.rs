//! Symmetric ciphers as OpenSSL's EVP layer offers them to
//! openssl_encrypt()/openssl_decrypt(): the names, key and IV lengths, the
//! block modes (ECB, CBC with PKCS#7 padding, CTR, CFB, CFB8, CFB1, OFB) and
//! the AEAD modes (GCM, CCM, ChaCha20-Poly1305), written over RustCrypto's
//! block ciphers so every IV and tag length OpenSSL accepts works here too.

use cipher::generic_array::GenericArray;
use cipher::{BlockDecrypt, BlockEncrypt, KeyInit, KeyIvInit, StreamCipher, StreamCipherSeek};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algo {
    Aes,
    Camellia,
    /// Three-key triple DES (des-ede3).
    Des3,
    /// Two-key triple DES (des-ede).
    Des2,
    ChaCha20,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ecb,
    Cbc,
    Ctr,
    Cfb,
    Cfb8,
    Cfb1,
    Ofb,
    Gcm,
    Ccm,
    Ocb,
    /// ChaCha20 alone (a stream cipher).
    Stream,
    ChaChaPoly,
}

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub name: &'static str,
    pub algo: Algo,
    pub mode: Mode,
    pub key_len: usize,
    pub iv_len: usize,
}

impl Spec {
    pub fn is_aead(&self) -> bool {
        matches!(self.mode, Mode::Gcm | Mode::Ccm | Mode::Ocb | Mode::ChaChaPoly)
    }

    /// Whether `len` bytes of IV are accepted (AEAD modes take several).
    pub fn iv_len_ok(&self, len: usize) -> bool {
        match self.mode {
            Mode::Gcm => (1..=128).contains(&len),
            Mode::Ccm => (7..=13).contains(&len),
            Mode::Ocb => (1..=15).contains(&len),
            Mode::ChaChaPoly => len == 12,
            _ => len == self.iv_len,
        }
    }
}

macro_rules! spec {
    ($name:literal, $algo:ident, $mode:ident, $key:literal, $iv:literal) => {
        Spec { name: $name, algo: Algo::$algo, mode: Mode::$mode, key_len: $key, iv_len: $iv }
    };
}

/// The ciphers, by the name openssl_get_cipher_methods() lists.
pub const SPECS: &[Spec] = &[
    spec!("aes-128-cbc", Aes, Cbc, 16, 16),
    spec!("aes-128-ccm", Aes, Ccm, 16, 12),
    spec!("aes-128-cfb", Aes, Cfb, 16, 16),
    spec!("aes-128-cfb1", Aes, Cfb1, 16, 16),
    spec!("aes-128-cfb8", Aes, Cfb8, 16, 16),
    spec!("aes-128-ctr", Aes, Ctr, 16, 16),
    spec!("aes-128-ecb", Aes, Ecb, 16, 0),
    spec!("aes-128-gcm", Aes, Gcm, 16, 12),
    spec!("aes-128-ocb", Aes, Ocb, 16, 12),
    spec!("aes-128-ofb", Aes, Ofb, 16, 16),
    spec!("aes-192-cbc", Aes, Cbc, 24, 16),
    spec!("aes-192-ccm", Aes, Ccm, 24, 12),
    spec!("aes-192-cfb", Aes, Cfb, 24, 16),
    spec!("aes-192-cfb1", Aes, Cfb1, 24, 16),
    spec!("aes-192-cfb8", Aes, Cfb8, 24, 16),
    spec!("aes-192-ctr", Aes, Ctr, 24, 16),
    spec!("aes-192-ecb", Aes, Ecb, 24, 0),
    spec!("aes-192-gcm", Aes, Gcm, 24, 12),
    spec!("aes-192-ocb", Aes, Ocb, 24, 12),
    spec!("aes-192-ofb", Aes, Ofb, 24, 16),
    spec!("aes-256-cbc", Aes, Cbc, 32, 16),
    spec!("aes-256-ccm", Aes, Ccm, 32, 12),
    spec!("aes-256-cfb", Aes, Cfb, 32, 16),
    spec!("aes-256-cfb1", Aes, Cfb1, 32, 16),
    spec!("aes-256-cfb8", Aes, Cfb8, 32, 16),
    spec!("aes-256-ctr", Aes, Ctr, 32, 16),
    spec!("aes-256-ecb", Aes, Ecb, 32, 0),
    spec!("aes-256-gcm", Aes, Gcm, 32, 12),
    spec!("aes-256-ocb", Aes, Ocb, 32, 12),
    spec!("aes-256-ofb", Aes, Ofb, 32, 16),
    spec!("camellia-128-cbc", Camellia, Cbc, 16, 16),
    spec!("camellia-128-cfb", Camellia, Cfb, 16, 16),
    spec!("camellia-128-cfb1", Camellia, Cfb1, 16, 16),
    spec!("camellia-128-cfb8", Camellia, Cfb8, 16, 16),
    spec!("camellia-128-ctr", Camellia, Ctr, 16, 16),
    spec!("camellia-128-ecb", Camellia, Ecb, 16, 0),
    spec!("camellia-128-ofb", Camellia, Ofb, 16, 16),
    spec!("camellia-192-cbc", Camellia, Cbc, 24, 16),
    spec!("camellia-192-cfb", Camellia, Cfb, 24, 16),
    spec!("camellia-192-cfb1", Camellia, Cfb1, 24, 16),
    spec!("camellia-192-cfb8", Camellia, Cfb8, 24, 16),
    spec!("camellia-192-ctr", Camellia, Ctr, 24, 16),
    spec!("camellia-192-ecb", Camellia, Ecb, 24, 0),
    spec!("camellia-192-ofb", Camellia, Ofb, 24, 16),
    spec!("camellia-256-cbc", Camellia, Cbc, 32, 16),
    spec!("camellia-256-cfb", Camellia, Cfb, 32, 16),
    spec!("camellia-256-cfb1", Camellia, Cfb1, 32, 16),
    spec!("camellia-256-cfb8", Camellia, Cfb8, 32, 16),
    spec!("camellia-256-ctr", Camellia, Ctr, 32, 16),
    spec!("camellia-256-ecb", Camellia, Ecb, 32, 0),
    spec!("camellia-256-ofb", Camellia, Ofb, 32, 16),
    spec!("chacha20", ChaCha20, Stream, 32, 16),
    spec!("chacha20-poly1305", ChaCha20, ChaChaPoly, 32, 12),
    spec!("des-ede-cbc", Des2, Cbc, 16, 8),
    spec!("des-ede-cfb", Des2, Cfb, 16, 8),
    spec!("des-ede-ecb", Des2, Ecb, 16, 0),
    spec!("des-ede-ofb", Des2, Ofb, 16, 8),
    spec!("des-ede3-cbc", Des3, Cbc, 24, 8),
    spec!("des-ede3-cfb", Des3, Cfb, 24, 8),
    spec!("des-ede3-cfb1", Des3, Cfb1, 24, 8),
    spec!("des-ede3-cfb8", Des3, Cfb8, 24, 8),
    spec!("des-ede3-ecb", Des3, Ecb, 24, 0),
    spec!("des-ede3-ofb", Des3, Ofb, 24, 8),
];

/// Other names OpenSSL knows them by (openssl_get_cipher_methods(true)).
const ALIASES: &[(&str, &str)] = &[
    ("aes128", "aes-128-cbc"),
    ("aes192", "aes-192-cbc"),
    ("aes256", "aes-256-cbc"),
    ("camellia128", "camellia-128-cbc"),
    ("camellia192", "camellia-192-cbc"),
    ("camellia256", "camellia-256-cbc"),
    ("des-ede", "des-ede-ecb"),
    ("des-ede3", "des-ede3-ecb"),
    ("des3", "des-ede3-cbc"),
    ("id-aes128-ccm", "aes-128-ccm"),
    ("id-aes128-gcm", "aes-128-gcm"),
    ("id-aes192-ccm", "aes-192-ccm"),
    ("id-aes192-gcm", "aes-192-gcm"),
    ("id-aes256-ccm", "aes-256-ccm"),
    ("id-aes256-gcm", "aes-256-gcm"),
    ("2.16.840.1.101.3.4.1.1", "aes-128-ecb"),
    ("2.16.840.1.101.3.4.1.2", "aes-128-cbc"),
    ("2.16.840.1.101.3.4.1.3", "aes-128-ofb"),
    ("2.16.840.1.101.3.4.1.4", "aes-128-cfb"),
    ("2.16.840.1.101.3.4.1.6", "aes-128-gcm"),
    ("2.16.840.1.101.3.4.1.7", "aes-128-ccm"),
    ("2.16.840.1.101.3.4.1.21", "aes-192-ecb"),
    ("2.16.840.1.101.3.4.1.22", "aes-192-cbc"),
    ("2.16.840.1.101.3.4.1.23", "aes-192-ofb"),
    ("2.16.840.1.101.3.4.1.24", "aes-192-cfb"),
    ("2.16.840.1.101.3.4.1.26", "aes-192-gcm"),
    ("2.16.840.1.101.3.4.1.27", "aes-192-ccm"),
    ("2.16.840.1.101.3.4.1.41", "aes-256-ecb"),
    ("2.16.840.1.101.3.4.1.42", "aes-256-cbc"),
    ("2.16.840.1.101.3.4.1.43", "aes-256-ofb"),
    ("2.16.840.1.101.3.4.1.44", "aes-256-cfb"),
    ("2.16.840.1.101.3.4.1.46", "aes-256-gcm"),
    ("2.16.840.1.101.3.4.1.47", "aes-256-ccm"),
    ("1.2.840.113549.3.7", "des-ede3-cbc"),
];

pub fn by_name(name: &[u8]) -> Option<Spec> {
    let name = std::str::from_utf8(name).ok()?;
    let canonical = ALIASES.iter().find(|(a, _)| a.eq_ignore_ascii_case(name)).map_or(name, |(_, c)| c);
    SPECS.iter().find(|s| s.name.eq_ignore_ascii_case(canonical)).copied()
}

/// openssl_get_cipher_methods()
pub fn method_names(aliases: bool) -> Vec<&'static str> {
    let mut out: Vec<&str> = SPECS.iter().map(|s| s.name).collect();
    if aliases {
        out.extend(ALIASES.iter().map(|(a, _)| *a));
        out.sort_by_key(|a| a.to_ascii_lowercase());
    }
    out
}

/// Why an operation failed, as OpenSSL's error queue words it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    BadDecrypt,
    WrongFinalBlockLength,
    /// An AEAD tag that does not match (OpenSSL queues no error for it).
    TagMismatch,
    /// CCM's tag check fails inside the update, which queues an error.
    CcmTag,
}

impl Failure {
    pub fn queued(self) -> Option<&'static str> {
        match self {
            Failure::BadDecrypt => Some("error:1C800064:Provider routines::bad decrypt"),
            Failure::WrongFinalBlockLength => Some("error:1C80006B:Provider routines::wrong final block length"),
            Failure::TagMismatch => None,
            Failure::CcmTag => Some("error:1C800066:Provider routines::cipher operation failed"),
        }
    }
}

// ---- block ciphers ----

enum Block {
    Aes128(Box<aes::Aes128>),
    Aes192(Box<aes::Aes192>),
    Aes256(Box<aes::Aes256>),
    Camellia128(Box<camellia::Camellia128>),
    Camellia192(Box<camellia::Camellia192>),
    Camellia256(Box<camellia::Camellia256>),
    Des3(Box<des::TdesEde3>),
    Des2(Box<des::TdesEde2>),
}

macro_rules! each_block {
    ($self:expr, $c:ident => $body:expr) => {
        match $self {
            Block::Aes128($c) => $body,
            Block::Aes192($c) => $body,
            Block::Aes256($c) => $body,
            Block::Camellia128($c) => $body,
            Block::Camellia192($c) => $body,
            Block::Camellia256($c) => $body,
            Block::Des3($c) => $body,
            Block::Des2($c) => $body,
        }
    };
}

impl Block {
    fn new(algo: Algo, key: &[u8]) -> Option<Block> {
        Some(match (algo, key.len()) {
            (Algo::Aes, 16) => Block::Aes128(Box::new(aes::Aes128::new_from_slice(key).ok()?)),
            (Algo::Aes, 24) => Block::Aes192(Box::new(aes::Aes192::new_from_slice(key).ok()?)),
            (Algo::Aes, 32) => Block::Aes256(Box::new(aes::Aes256::new_from_slice(key).ok()?)),
            (Algo::Camellia, 16) => Block::Camellia128(Box::new(camellia::Camellia128::new_from_slice(key).ok()?)),
            (Algo::Camellia, 24) => Block::Camellia192(Box::new(camellia::Camellia192::new_from_slice(key).ok()?)),
            (Algo::Camellia, 32) => Block::Camellia256(Box::new(camellia::Camellia256::new_from_slice(key).ok()?)),
            (Algo::Des3, 24) => Block::Des3(Box::new(des::TdesEde3::new_from_slice(key).ok()?)),
            (Algo::Des2, 16) => Block::Des2(Box::new(des::TdesEde2::new_from_slice(key).ok()?)),
            _ => return None,
        })
    }

    fn size(&self) -> usize {
        match self {
            Block::Des3(_) | Block::Des2(_) => 8,
            _ => 16,
        }
    }

    fn encrypt(&self, block: &mut [u8]) {
        each_block!(self, c => c.encrypt_block(GenericArray::from_mut_slice(block)))
    }

    fn decrypt(&self, block: &mut [u8]) {
        each_block!(self, c => c.decrypt_block(GenericArray::from_mut_slice(block)))
    }
}

/// What openssl_encrypt()/openssl_decrypt() give the cipher once PHP has
/// fixed up the key and IV lengths.
pub struct Job<'a> {
    pub spec: Spec,
    pub key: &'a [u8],
    pub iv: &'a [u8],
    pub data: &'a [u8],
    pub aad: &'a [u8],
    /// Padding (ECB and CBC); off with OPENSSL_ZERO_PADDING.
    pub pad: bool,
    /// The tag length to make (encrypting) or the tag to check (decrypting).
    pub tag_len: usize,
    pub tag: &'a [u8],
}

/// Encrypt: the ciphertext and, for an AEAD mode, the full tag.
pub fn encrypt(job: &Job) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    run(job, true)
}

pub fn decrypt(job: &Job) -> Result<Vec<u8>, Failure> {
    run(job, false).map(|(out, _)| out)
}

fn run(job: &Job, enc: bool) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    let spec = job.spec;
    if spec.algo == Algo::ChaCha20 {
        return chacha(job, enc);
    }
    let block = Block::new(spec.algo, job.key).ok_or(Failure::BadDecrypt)?;
    let bs = block.size();
    let mut iv = job.iv.to_vec();
    match spec.mode {
        Mode::Ecb | Mode::Cbc => {
            let cbc = spec.mode == Mode::Cbc;
            if enc {
                let mut data = job.data.to_vec();
                if job.pad {
                    let n = bs - data.len() % bs;
                    data.extend(std::iter::repeat_n(n as u8, n));
                } else if !data.len().is_multiple_of(bs) {
                    return Err(Failure::WrongFinalBlockLength);
                }
                for chunk in data.chunks_mut(bs) {
                    if cbc {
                        xor(chunk, &iv);
                    }
                    block.encrypt(chunk);
                    if cbc {
                        iv.copy_from_slice(chunk);
                    }
                }
                Ok((data, Vec::new()))
            } else {
                let mut data = job.data.to_vec();
                if !data.len().is_multiple_of(bs) || (job.pad && data.is_empty()) {
                    return Err(Failure::WrongFinalBlockLength);
                }
                for chunk in data.chunks_mut(bs) {
                    let saved = chunk.to_vec();
                    block.decrypt(chunk);
                    if cbc {
                        xor(chunk, &iv);
                        iv = saved;
                    }
                }
                if job.pad {
                    let n = *data.last().unwrap_or(&0) as usize;
                    if n == 0 || n > bs || data[data.len() - n..].iter().any(|b| *b as usize != n) {
                        return Err(Failure::BadDecrypt);
                    }
                    data.truncate(data.len() - n);
                }
                Ok((data, Vec::new()))
            }
        }
        Mode::Ctr => {
            let mut out = job.data.to_vec();
            let mut counter = iv.clone();
            for chunk in out.chunks_mut(bs) {
                let mut ks = counter.clone();
                block.encrypt(&mut ks);
                xor(chunk, &ks);
                increment(&mut counter);
            }
            Ok((out, Vec::new()))
        }
        Mode::Ofb => {
            let mut out = job.data.to_vec();
            for chunk in out.chunks_mut(bs) {
                block.encrypt(&mut iv);
                xor(chunk, &iv);
            }
            Ok((out, Vec::new()))
        }
        Mode::Cfb => {
            let mut out = job.data.to_vec();
            for chunk in out.chunks_mut(bs) {
                let mut ks = iv.clone();
                block.encrypt(&mut ks);
                let input = chunk.to_vec();
                xor(chunk, &ks);
                let fed = if enc { chunk.to_vec() } else { input };
                iv[..fed.len()].copy_from_slice(&fed);
            }
            Ok((out, Vec::new()))
        }
        Mode::Cfb8 => {
            let mut out = job.data.to_vec();
            for byte in out.iter_mut() {
                let mut ks = iv.clone();
                block.encrypt(&mut ks);
                let input = *byte;
                *byte ^= ks[0];
                let fed = if enc { *byte } else { input };
                iv.rotate_left(1);
                *iv.last_mut().expect("an IV") = fed;
            }
            Ok((out, Vec::new()))
        }
        Mode::Cfb1 => {
            let mut out = job.data.to_vec();
            for byte in out.iter_mut() {
                let mut result = 0u8;
                for bit in (0..8).rev() {
                    let mut ks = iv.clone();
                    block.encrypt(&mut ks);
                    let input = (*byte >> bit) & 1;
                    let output = input ^ (ks[0] >> 7);
                    result |= output << bit;
                    shift_in_bit(&mut iv, if enc { output } else { input });
                }
                *byte = result;
            }
            Ok((out, Vec::new()))
        }
        Mode::Gcm => gcm(&block, job, enc),
        Mode::Ccm => ccm(&block, job, enc),
        Mode::Ocb => ocb(&block, job, enc),
        Mode::Stream | Mode::ChaChaPoly => unreachable!("ChaCha20 is handled above"),
    }
}

fn xor(a: &mut [u8], b: &[u8]) {
    for (x, y) in a.iter_mut().zip(b) {
        *x ^= y;
    }
}

/// A big-endian counter over the whole block, as OpenSSL's CTR mode counts.
fn increment(counter: &mut [u8]) {
    for b in counter.iter_mut().rev() {
        *b = b.wrapping_add(1);
        if *b != 0 {
            break;
        }
    }
}

fn shift_in_bit(iv: &mut [u8], bit: u8) {
    let n = iv.len();
    for i in 0..n {
        let next = if i + 1 < n { iv[i + 1] >> 7 } else { bit };
        iv[i] = (iv[i] << 1) | next;
    }
}

// ---- GCM ----

fn ghash(h: &[u8; 16], aad: &[u8], ct: &[u8]) -> [u8; 16] {
    use universal_hash::UniversalHash;
    let mut g = ghash::GHash::new(GenericArray::from_slice(h));
    g.update_padded(aad);
    g.update_padded(ct);
    let mut lens = [0u8; 16];
    lens[..8].copy_from_slice(&((aad.len() as u64) * 8).to_be_bytes());
    lens[8..].copy_from_slice(&((ct.len() as u64) * 8).to_be_bytes());
    g.update(&[*GenericArray::from_slice(&lens)]);
    g.finalize().into()
}

fn gcm(block: &Block, job: &Job, enc: bool) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    let mut h = [0u8; 16];
    block.encrypt(&mut h);
    let j0: [u8; 16] = if job.iv.len() == 12 {
        let mut j = [0u8; 16];
        j[..12].copy_from_slice(job.iv);
        j[15] = 1;
        j
    } else {
        ghash(&h, &[], job.iv)
    };
    let mut counter = j0;
    let mut out = job.data.to_vec();
    for chunk in out.chunks_mut(16) {
        inc32(&mut counter);
        let mut ks = counter;
        block.encrypt(&mut ks);
        xor(chunk, &ks);
    }
    let ct = if enc { &out[..] } else { job.data };
    let s = ghash(&h, job.aad, ct);
    let mut tag = j0;
    block.encrypt(&mut tag);
    xor(&mut tag, &s);
    if enc {
        return Ok((out, tag.to_vec()));
    }
    if job.tag.is_empty() || job.tag.len() > 16 || !constant_eq(&tag[..job.tag.len()], job.tag) {
        return Err(Failure::TagMismatch);
    }
    Ok((out, Vec::new()))
}

fn inc32(counter: &mut [u8; 16]) {
    let n = u32::from_be_bytes([counter[12], counter[13], counter[14], counter[15]]).wrapping_add(1);
    counter[12..].copy_from_slice(&n.to_be_bytes());
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---- CCM (RFC 3610) ----

fn ccm(block: &Block, job: &Job, enc: bool) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    let nonce = job.iv;
    let l = 15 - nonce.len();
    let m = if enc { job.tag_len } else { job.tag.len() };
    let msg_len = job.data.len();
    if l < 8 && (msg_len as u64) >> (8 * l) != 0 {
        return Err(Failure::BadDecrypt);
    }
    let ctr_block = |i: u64| {
        let mut a = [0u8; 16];
        a[0] = (l - 1) as u8;
        a[1..1 + nonce.len()].copy_from_slice(nonce);
        let ib = i.to_be_bytes();
        a[16 - l..].copy_from_slice(&ib[8 - l..]);
        a
    };
    // CTR with counters from 1 over the payload.
    let mut out = job.data.to_vec();
    for (i, chunk) in out.chunks_mut(16).enumerate() {
        let mut ks = ctr_block(i as u64 + 1);
        block.encrypt(&mut ks);
        xor(chunk, &ks);
    }
    let plain = if enc { job.data } else { &out[..] };
    // CBC-MAC over B0, the encoded AAD and the payload.
    let mut b0 = [0u8; 16];
    b0[0] = (if job.aad.is_empty() { 0 } else { 0x40 }) | ((((m.max(2) - 2) / 2) as u8) << 3) | (l - 1) as u8;
    b0[1..1 + nonce.len()].copy_from_slice(nonce);
    let lb = (msg_len as u64).to_be_bytes();
    b0[16 - l..].copy_from_slice(&lb[8 - l..]);
    let mut mac = b0;
    block.encrypt(&mut mac);
    let absorb = |data: &[u8], mac: &mut [u8; 16]| {
        for chunk in data.chunks(16) {
            xor(mac, chunk);
            block.encrypt(mac);
        }
    };
    if !job.aad.is_empty() {
        let a = job.aad.len();
        let mut enc_aad = if a < 0xff00 {
            (a as u16).to_be_bytes().to_vec()
        } else if (a as u64) < (1 << 32) {
            [&[0xff, 0xfe][..], &(a as u32).to_be_bytes()].concat()
        } else {
            [&[0xff, 0xff][..], &(a as u64).to_be_bytes()].concat()
        };
        enc_aad.extend_from_slice(job.aad);
        absorb(&enc_aad, &mut mac);
    }
    absorb(plain, &mut mac);
    let mut s0 = ctr_block(0);
    block.encrypt(&mut s0);
    xor(&mut mac, &s0);
    let tag = mac[..m].to_vec();
    if enc {
        return Ok((out, tag));
    }
    if !constant_eq(&tag, job.tag) {
        return Err(Failure::CcmTag);
    }
    Ok((out, Vec::new()))
}

/// Whether CCM takes a tag of `len` bytes.
pub fn ccm_tag_len_ok(len: usize) -> bool {
    (4..=16).contains(&len) && len.is_multiple_of(2)
}

// ---- OCB (RFC 7253) ----

fn ocb_double(s: &[u8; 16]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = (s[i] << 1) | if i + 1 < 16 { s[i + 1] >> 7 } else { 0 };
    }
    if s[0] & 0x80 != 0 {
        out[15] ^= 0x87;
    }
    out
}

struct OcbKey<'a> {
    block: &'a Block,
    l_star: [u8; 16],
    l_dollar: [u8; 16],
    l: Vec<[u8; 16]>,
}

impl OcbKey<'_> {
    fn new(block: &Block) -> OcbKey<'_> {
        let mut l_star = [0u8; 16];
        block.encrypt(&mut l_star);
        let l_dollar = ocb_double(&l_star);
        let mut l = vec![ocb_double(&l_dollar)];
        for _ in 0..63 {
            let next = ocb_double(l.last().expect("L_0"));
            l.push(next);
        }
        OcbKey { block, l_star, l_dollar, l }
    }

    fn enc(&self, b: &[u8; 16]) -> [u8; 16] {
        let mut out = *b;
        self.block.encrypt(&mut out);
        out
    }

    fn hash(&self, aad: &[u8]) -> [u8; 16] {
        let mut sum = [0u8; 16];
        let mut offset = [0u8; 16];
        let (blocks, rest) = aad.as_chunks::<16>();
        for (i, a) in blocks.iter().enumerate() {
            xor(&mut offset, &self.l[(i + 1).trailing_zeros() as usize]);
            let mut x = offset;
            xor(&mut x, a);
            xor(&mut sum, &self.enc(&x));
        }
        if !rest.is_empty() {
            xor(&mut offset, &self.l_star);
            let mut x = [0u8; 16];
            x[..rest.len()].copy_from_slice(rest);
            x[rest.len()] = 0x80;
            xor(&mut x, &offset);
            xor(&mut sum, &self.enc(&x));
        }
        sum
    }
}

fn ocb(block: &Block, job: &Job, enc: bool) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    let k = OcbKey::new(block);
    let tag_len = if enc { job.tag_len } else { job.tag.len() };
    let n = job.iv;
    let mut nonce = [0u8; 16];
    nonce[0] = (((tag_len * 8) % 128) as u8) << 1;
    nonce[16 - n.len()..].copy_from_slice(n);
    nonce[15 - n.len()] |= 1;
    let bottom = (nonce[15] & 0x3f) as usize;
    let mut top = nonce;
    top[15] &= 0xc0;
    let ktop = k.enc(&top);
    let mut stretch = [0u8; 24];
    stretch[..16].copy_from_slice(&ktop);
    for i in 0..8 {
        stretch[16 + i] = ktop[i] ^ ktop[i + 1];
    }
    let mut offset = [0u8; 16];
    let (byte, bit) = (bottom / 8, bottom % 8);
    for i in 0..16 {
        offset[i] = if bit == 0 { stretch[i + byte] } else { (stretch[i + byte] << bit) | (stretch[i + byte + 1] >> (8 - bit)) };
    }
    let mut checksum = [0u8; 16];
    let mut out = Vec::with_capacity(job.data.len());
    let (blocks, rest) = job.data.as_chunks::<16>();
    for (i, c) in blocks.iter().enumerate() {
        xor(&mut offset, &k.l[(i + 1).trailing_zeros() as usize]);
        let mut x = offset;
        xor(&mut x, c);
        if enc {
            block.encrypt(&mut x);
            xor(&mut checksum, c);
        } else {
            block.decrypt(&mut x);
        }
        xor(&mut x, &offset);
        if !enc {
            xor(&mut checksum, &x);
        }
        out.extend_from_slice(&x);
    }
    if !rest.is_empty() {
        xor(&mut offset, &k.l_star);
        let pad = k.enc(&offset);
        let mut part = rest.to_vec();
        xor(&mut part, &pad);
        let plain = if enc { rest } else { &part[..] };
        let mut padded = [0u8; 16];
        padded[..plain.len()].copy_from_slice(plain);
        padded[plain.len()] = 0x80;
        xor(&mut checksum, &padded);
        out.extend_from_slice(&part);
    }
    let mut t = checksum;
    xor(&mut t, &offset);
    xor(&mut t, &k.l_dollar);
    let mut tag = k.enc(&t);
    xor(&mut tag, &k.hash(job.aad));
    if enc {
        return Ok((out, tag[..tag_len].to_vec()));
    }
    if job.tag.is_empty() || !constant_eq(&tag[..tag_len], job.tag) {
        return Err(Failure::TagMismatch);
    }
    Ok((out, Vec::new()))
}

// ---- ChaCha20 and ChaCha20-Poly1305 ----

/// OpenSSL's 16-byte ChaCha20 IV: a little-endian block counter, then the nonce.
fn chacha_at(key: &[u8], iv16: &[u8; 16]) -> chacha20::ChaCha20 {
    let counter = u32::from_le_bytes([iv16[0], iv16[1], iv16[2], iv16[3]]);
    let mut c = chacha20::ChaCha20::new(GenericArray::from_slice(key), GenericArray::from_slice(&iv16[4..]));
    c.seek(u64::from(counter) * 64);
    c
}

fn chacha(job: &Job, enc: bool) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    let mut iv16 = [0u8; 16];
    let n = job.iv.len().min(16);
    iv16[16 - n..].copy_from_slice(&job.iv[..n]);
    if job.spec.mode == Mode::Stream {
        let mut iv16 = [0u8; 16];
        iv16[..n].copy_from_slice(&job.iv[..n]);
        let mut out = job.data.to_vec();
        chacha_at(job.key, &iv16).apply_keystream(&mut out);
        return Ok((out, Vec::new()));
    }
    // RFC 8439: the Poly1305 key is block 0's keystream; the data starts at block 1.
    use universal_hash::UniversalHash;
    let mut c = chacha_at(job.key, &iv16);
    let mut otk = [0u8; 64];
    c.apply_keystream(&mut otk);
    let mut out = job.data.to_vec();
    c.apply_keystream(&mut out);
    let ct = if enc { &out[..] } else { job.data };
    let mut mac = poly1305::Poly1305::new(GenericArray::from_slice(&otk[..32]));
    mac.update_padded(job.aad);
    mac.update_padded(ct);
    let mut lens = [0u8; 16];
    lens[..8].copy_from_slice(&(job.aad.len() as u64).to_le_bytes());
    lens[8..].copy_from_slice(&(ct.len() as u64).to_le_bytes());
    mac.update(&[*GenericArray::from_slice(&lens)]);
    let tag: [u8; 16] = mac.finalize().into();
    if enc {
        return Ok((out, tag.to_vec()));
    }
    if job.tag.is_empty() || job.tag.len() > 16 || !constant_eq(&tag[..job.tag.len()], job.tag) {
        return Err(Failure::TagMismatch);
    }
    Ok((out, Vec::new()))
}

/// A one-shot encryption with a cipher by name (key and IV of its
/// lengths), for the key formats that encrypt with PEM or PKCS#5.
pub fn simple(name: &str, key: &[u8], iv: &[u8], data: &[u8], enc: bool) -> Option<Vec<u8>> {
    let spec = by_name(name.as_bytes())?;
    let job = Job { spec, key, iv, data, aad: &[], pad: true, tag_len: 0, tag: &[] };
    if enc { encrypt(&job).ok().map(|(o, _)| o) } else { decrypt(&job).ok() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn gcm_matches_the_nist_vector() {
        // GCM spec test case 3 (AES-128, 96-bit IV).
        let spec = by_name(b"aes-128-gcm").unwrap();
        let key = hex("feffe9928665731c6d6a8f9467308308");
        let iv = hex("cafebabefacedbaddecaf888");
        let pt = hex("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255");
        let job = Job { spec, key: &key, iv: &iv, data: &pt, aad: &[], pad: true, tag_len: 16, tag: &[] };
        let (ct, tag) = encrypt(&job).unwrap();
        assert_eq!(crate::der::hex_lower(&tag), "4d5c2af327cd64a62cf35abd2ba6fab4");
        let job = Job { spec, key: &key, iv: &iv, data: &ct, aad: &[], pad: true, tag_len: 16, tag: &tag };
        assert_eq!(decrypt(&job).unwrap(), pt);
    }

    #[test]
    fn ocb_matches_rfc7253() {
        let spec = by_name(b"aes-128-ocb").unwrap();
        let key = hex("000102030405060708090A0B0C0D0E0F");
        let iv = hex("BBAA99887766554433221101");
        let ab = hex("0001020304050607");
        let job = Job { spec, key: &key, iv: &iv, data: &ab, aad: &ab, pad: true, tag_len: 16, tag: &[] };
        let (ct, tag) = encrypt(&job).unwrap();
        assert_eq!(crate::der::hex_upper(&ct, ""), "6820B3657B6F615A");
        assert_eq!(crate::der::hex_upper(&tag, ""), "5725BDA0D3B4EB3A257C9AF1F8F03009");
        let job = Job { data: &ct, tag: &tag, ..job };
        assert_eq!(decrypt(&job).unwrap(), ab);
    }

    #[test]
    fn cbc_round_trips_with_padding() {
        let spec = by_name(b"AES-256-CBC").unwrap();
        let key = [7u8; 32];
        let iv = [1u8; 16];
        let job = Job { spec, key: &key, iv: &iv, data: b"hello world", aad: &[], pad: true, tag_len: 0, tag: &[] };
        let (ct, _) = encrypt(&job).unwrap();
        assert_eq!(ct.len(), 16);
        let job = Job { data: &ct, ..job };
        assert_eq!(decrypt(&job).unwrap(), b"hello world");
    }
}
