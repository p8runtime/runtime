//! Message digests: OpenSSL's names and aliases for them (looked up
//! without regard to case, as OpenSSL 3 does), hashing, HMAC and PBKDF2.

use digest::{Digest, ExtendableOutput, Update};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Md {
    Md4,
    Md5,
    Ssl3Md5Sha1,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
    Sha512_224,
    Sha512_256,
    Sha3_224,
    Sha3_256,
    Sha3_384,
    Sha3_512,
    Shake128,
    Shake256,
    Ripemd160,
    Whirlpool,
    Blake2b512,
    Blake2s256,
    Sm3,
}

/// (name as openssl_get_md_methods() lists it, digest, aliases).
const NAMES: &[(&str, Md, &[&str])] = &[
    ("blake2b512", Md::Blake2b512, &["BLAKE2B-512"]),
    ("blake2s256", Md::Blake2s256, &["BLAKE2S-256"]),
    ("md4", Md::Md4, &["RSA-MD4", "md4WithRSAEncryption"]),
    ("md5", Md::Md5, &["RSA-MD5", "md5WithRSAEncryption", "ssl3-md5"]),
    ("md5-sha1", Md::Ssl3Md5Sha1, &[]),
    ("ripemd160", Md::Ripemd160, &["RSA-RIPEMD160", "ripemd", "ripemd160WithRSA", "rmd160", "RIPEMD-160"]),
    ("sha1", Md::Sha1, &["RSA-SHA1", "RSA-SHA1-2", "sha1WithRSAEncryption", "ssl3-sha1", "SHA-1"]),
    ("sha224", Md::Sha224, &["RSA-SHA224", "sha224WithRSAEncryption", "SHA2-224", "SHA-224"]),
    ("sha256", Md::Sha256, &["RSA-SHA256", "sha256WithRSAEncryption", "SHA2-256", "SHA-256"]),
    ("sha3-224", Md::Sha3_224, &["RSA-SHA3-224", "id-rsassa-pkcs1-v1_5-with-sha3-224"]),
    ("sha3-256", Md::Sha3_256, &["RSA-SHA3-256", "id-rsassa-pkcs1-v1_5-with-sha3-256"]),
    ("sha3-384", Md::Sha3_384, &["RSA-SHA3-384", "id-rsassa-pkcs1-v1_5-with-sha3-384"]),
    ("sha3-512", Md::Sha3_512, &["RSA-SHA3-512", "id-rsassa-pkcs1-v1_5-with-sha3-512"]),
    ("sha384", Md::Sha384, &["RSA-SHA384", "sha384WithRSAEncryption", "SHA2-384", "SHA-384"]),
    ("sha512", Md::Sha512, &["RSA-SHA512", "sha512WithRSAEncryption", "SHA2-512", "SHA-512"]),
    ("sha512-224", Md::Sha512_224, &["RSA-SHA512/224", "sha512-224WithRSAEncryption", "SHA2-512/224", "SHA-512/224"]),
    ("sha512-256", Md::Sha512_256, &["RSA-SHA512/256", "sha512-256WithRSAEncryption", "SHA2-512/256", "SHA-512/256"]),
    ("shake128", Md::Shake128, &["SHAKE-128"]),
    ("shake256", Md::Shake256, &["SHAKE-256"]),
    ("sm3", Md::Sm3, &["RSA-SM3", "sm3WithRSAEncryption"]),
    ("whirlpool", Md::Whirlpool, &[]),
];

/// The aliases openssl_get_md_methods(true) adds (those OpenSSL lists).
const LISTED_ALIASES: &[&str] = &[
    "RSA-MD4", "RSA-MD5", "RSA-RIPEMD160", "RSA-SHA1", "RSA-SHA1-2", "RSA-SHA224", "RSA-SHA256", "RSA-SHA3-224",
    "RSA-SHA3-256", "RSA-SHA3-384", "RSA-SHA3-512", "RSA-SHA384", "RSA-SHA512", "RSA-SHA512/224", "RSA-SHA512/256",
    "RSA-SM3", "id-rsassa-pkcs1-v1_5-with-sha3-224", "id-rsassa-pkcs1-v1_5-with-sha3-256",
    "id-rsassa-pkcs1-v1_5-with-sha3-384", "id-rsassa-pkcs1-v1_5-with-sha3-512", "md4WithRSAEncryption",
    "md5WithRSAEncryption", "ripemd", "ripemd160WithRSA", "rmd160", "sha1WithRSAEncryption",
    "sha224WithRSAEncryption", "sha256WithRSAEncryption", "sha384WithRSAEncryption", "sha512-224WithRSAEncryption",
    "sha512-256WithRSAEncryption", "sha512WithRSAEncryption", "sm3WithRSAEncryption", "ssl3-md5", "ssl3-sha1",
];

pub fn by_name(name: &[u8]) -> Option<Md> {
    let name = std::str::from_utf8(name).ok()?;
    for (n, md, aliases) in NAMES {
        if n.eq_ignore_ascii_case(name) || aliases.iter().any(|a| a.eq_ignore_ascii_case(name)) {
            return Some(*md);
        }
    }
    // A digest's object identifier also names it.
    let oid = match name {
        "1.2.840.113549.2.4" => Md::Md4,
        "1.2.840.113549.2.5" => Md::Md5,
        "1.3.14.3.2.26" => Md::Sha1,
        "2.16.840.1.101.3.4.2.1" => Md::Sha256,
        "2.16.840.1.101.3.4.2.2" => Md::Sha384,
        "2.16.840.1.101.3.4.2.3" => Md::Sha512,
        "2.16.840.1.101.3.4.2.4" => Md::Sha224,
        _ => return None,
    };
    Some(oid)
}

/// openssl_get_md_methods()
pub fn method_names(aliases: bool) -> Vec<&'static str> {
    let mut out: Vec<&str> = NAMES.iter().map(|(n, ..)| *n).collect();
    if aliases {
        out.extend_from_slice(LISTED_ALIASES);
        out.sort_by_key(|a| a.to_ascii_lowercase());
    }
    out
}

macro_rules! pbkdf2_typed {
    ($d:ty, $password:expr, $salt:expr, $iterations:expr, $len:expr) => {{
        let mut out = vec![0u8; $len];
        pbkdf2::pbkdf2_hmac::<$d>($password, $salt, $iterations, &mut out);
        out
    }};
}

impl Md {
    pub fn size(self) -> usize {
        match self {
            Md::Md4 | Md::Md5 | Md::Shake128 => 16,
            Md::Ssl3Md5Sha1 => 36,
            Md::Sha1 | Md::Ripemd160 => 20,
            Md::Sha224 | Md::Sha512_224 | Md::Sha3_224 => 28,
            Md::Sha256 | Md::Sha512_256 | Md::Sha3_256 | Md::Shake256 | Md::Blake2s256 | Md::Sm3 => 32,
            Md::Sha384 | Md::Sha3_384 => 48,
            Md::Sha512 | Md::Sha3_512 | Md::Whirlpool | Md::Blake2b512 => 64,
        }
    }

    pub fn hash(self, data: &[u8]) -> Vec<u8> {
        fn h<D: Digest>(data: &[u8]) -> Vec<u8> {
            D::digest(data).to_vec()
        }
        match self {
            Md::Md4 => h::<md4::Md4>(data),
            Md::Md5 => h::<md5::Md5>(data),
            Md::Ssl3Md5Sha1 => [h::<md5::Md5>(data), h::<sha1::Sha1>(data)].concat(),
            Md::Sha1 => h::<sha1::Sha1>(data),
            Md::Sha224 => h::<sha2::Sha224>(data),
            Md::Sha256 => h::<sha2::Sha256>(data),
            Md::Sha384 => h::<sha2::Sha384>(data),
            Md::Sha512 => h::<sha2::Sha512>(data),
            Md::Sha512_224 => h::<sha2::Sha512_224>(data),
            Md::Sha512_256 => h::<sha2::Sha512_256>(data),
            Md::Sha3_224 => h::<sha3::Sha3_224>(data),
            Md::Sha3_256 => h::<sha3::Sha3_256>(data),
            Md::Sha3_384 => h::<sha3::Sha3_384>(data),
            Md::Sha3_512 => h::<sha3::Sha3_512>(data),
            Md::Shake128 => {
                let mut x = sha3::Shake128::default();
                x.update(data);
                x.finalize_boxed(16).to_vec()
            }
            Md::Shake256 => {
                let mut x = sha3::Shake256::default();
                x.update(data);
                x.finalize_boxed(32).to_vec()
            }
            Md::Ripemd160 => h::<ripemd::Ripemd160>(data),
            Md::Whirlpool => h::<whirlpool::Whirlpool>(data),
            Md::Blake2b512 => h::<blake2::Blake2b512>(data),
            Md::Blake2s256 => h::<blake2::Blake2s256>(data),
            Md::Sm3 => h::<sm3::Sm3>(data),
        }
    }

    /// The block size HMAC pads its key to (0: no HMAC with this digest).
    fn block_size(self) -> usize {
        match self {
            Md::Md4 | Md::Md5 | Md::Ssl3Md5Sha1 | Md::Sha1 | Md::Sha224 | Md::Sha256 | Md::Ripemd160 | Md::Whirlpool | Md::Sm3 | Md::Blake2s256 => 64,
            Md::Sha384 | Md::Sha512 | Md::Sha512_224 | Md::Sha512_256 | Md::Blake2b512 => 128,
            Md::Sha3_224 => 144,
            Md::Sha3_256 => 136,
            Md::Sha3_384 => 104,
            Md::Sha3_512 => 72,
            Md::Shake128 | Md::Shake256 => 0,
        }
    }

    /// HMAC (RFC 2104) with this digest.
    pub fn hmac(self, key: &[u8], data: &[u8]) -> Vec<u8> {
        let block = self.block_size().max(1);
        let mut k = if key.len() > block { self.hash(key) } else { key.to_vec() };
        k.resize(block, 0);
        let mut inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
        inner.extend_from_slice(data);
        let mut outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
        outer.extend_from_slice(&self.hash(&inner));
        self.hash(&outer)
    }

    /// Whether OpenSSL 3's default provider computes it: MD4 and Whirlpool
    /// live in the legacy one, and the XOFs need a length.
    pub fn usable(self) -> bool {
        !matches!(self, Md::Md4 | Md::Whirlpool | Md::Shake128 | Md::Shake256)
    }

    pub fn has_hmac(self) -> bool {
        self.block_size() > 0
    }

    /// PBKDF2 (RFC 8018) with HMAC over this digest.
    pub fn pbkdf2(self, password: &[u8], salt: &[u8], iterations: u32, len: usize) -> Vec<u8> {
        match self {
            Md::Sha1 => pbkdf2_typed!(sha1::Sha1, password, salt, iterations, len),
            Md::Sha256 => pbkdf2_typed!(sha2::Sha256, password, salt, iterations, len),
            Md::Sha512 => pbkdf2_typed!(sha2::Sha512, password, salt, iterations, len),
            _ => {
                let mut out = Vec::with_capacity(len);
                let mut block = 1u32;
                while out.len() < len {
                    let mut u = self.hmac(password, &[salt, &block.to_be_bytes()].concat());
                    let mut t = u.clone();
                    for _ in 1..iterations {
                        u = self.hmac(password, &u);
                        for (a, b) in t.iter_mut().zip(&u) {
                            *a ^= b;
                        }
                    }
                    out.extend_from_slice(&t);
                    block += 1;
                }
                out.truncate(len);
                out
            }
        }
    }

    /// The DER DigestInfo prefix a PKCS#1 v1.5 signature puts before the
    /// hash (None: the hash goes in alone, as for md5-sha1).
    pub fn digest_info_prefix(self) -> Option<Vec<u8>> {
        use crate::der;
        let oid = self.oid()?;
        let alg = der::seq(&[&der::oid(oid), &der::null()]);
        let full = der::seq(&[&alg, &der::octet_string(&vec![0; self.size()])]);
        Some(full[..full.len() - self.size()].to_vec())
    }

    pub fn oid(self) -> Option<&'static str> {
        Some(match self {
            Md::Md4 => "1.2.840.113549.2.4",
            Md::Md5 => "1.2.840.113549.2.5",
            Md::Sha1 => "1.3.14.3.2.26",
            Md::Sha224 => "2.16.840.1.101.3.4.2.4",
            Md::Sha256 => "2.16.840.1.101.3.4.2.1",
            Md::Sha384 => "2.16.840.1.101.3.4.2.2",
            Md::Sha512 => "2.16.840.1.101.3.4.2.3",
            Md::Sha512_224 => "2.16.840.1.101.3.4.2.5",
            Md::Sha512_256 => "2.16.840.1.101.3.4.2.6",
            Md::Sha3_224 => "2.16.840.1.101.3.4.2.7",
            Md::Sha3_256 => "2.16.840.1.101.3.4.2.8",
            Md::Sha3_384 => "2.16.840.1.101.3.4.2.9",
            Md::Sha3_512 => "2.16.840.1.101.3.4.2.10",
            Md::Ripemd160 => "1.3.36.3.2.1",
            Md::Sm3 => "1.2.156.10197.1.401",
            _ => return None,
        })
    }

}


/// OPENSSL_ALGO_* constants.
pub fn by_algo(algo: i64) -> Option<Md> {
    Some(match algo {
        1 => Md::Sha1,
        2 => Md::Md5,
        3 => Md::Md4,
        6 => Md::Sha224,
        7 => Md::Sha256,
        8 => Md::Sha384,
        9 => Md::Sha512,
        10 => Md::Ripemd160,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc4231() {
        let mac = Md::Sha256.hmac(&[0x0b; 20], b"Hi There");
        assert_eq!(crate::der::hex_lower(&mac), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    }

    #[test]
    fn generic_pbkdf2_matches_the_typed_one() {
        let a = Md::Sha1.pbkdf2(b"password", b"salt", 2, 20);
        let mut b = vec![0u8; 20];
        let mut block = 1u32.to_be_bytes().to_vec();
        block.splice(0..0, b"salt".iter().copied());
        let u1 = Md::Sha1.hmac(b"password", &block);
        let u2 = Md::Sha1.hmac(b"password", &u1);
        for i in 0..20 {
            b[i] = u1[i] ^ u2[i];
        }
        assert_eq!(a, b);
        assert_eq!(crate::der::hex_lower(&a), "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957");
    }
}
