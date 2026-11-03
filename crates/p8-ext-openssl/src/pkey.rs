//! Asymmetric keys: RSA, EC (P-256, P-384, P-521, secp256k1), Ed25519 and
//! X25519, read from and written to the PEM forms OpenSSL 3 reads and
//! writes (PKCS#8, encrypted PKCS#8, PKCS#1, SEC1, SubjectPublicKeyInfo,
//! legacy PEM encryption), and what is done with them: signing, verifying,
//! RSA encryption with each padding, key agreement, generation.

use crate::der;
use crate::digest::Md;
use rand_core::{OsRng, RngCore};
use rsa::traits::{PrivateKeyParts, PublicKeyParts};
use rsa::{BigUint, RsaPrivateKey, RsaPublicKey};

pub const OID_RSA: &str = "1.2.840.113549.1.1.1";
pub const OID_EC: &str = "1.2.840.10045.2.1";
pub const OID_ED25519: &str = "1.3.101.112";
pub const OID_X25519: &str = "1.3.101.110";

/// OPENSSL_KEYTYPE_*
pub const KEYTYPE_RSA: i64 = 0;
pub const KEYTYPE_DSA: i64 = 1;
pub const KEYTYPE_DH: i64 = 2;
pub const KEYTYPE_EC: i64 = 3;
pub const KEYTYPE_X25519: i64 = 4;
pub const KEYTYPE_ED25519: i64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    P256,
    P384,
    P521,
    K256,
}

pub const CURVES: &[(Curve, &str, &str)] = &[
    (Curve::K256, "secp256k1", "1.3.132.0.10"),
    (Curve::P384, "secp384r1", "1.3.132.0.34"),
    (Curve::P521, "secp521r1", "1.3.132.0.35"),
    (Curve::P256, "prime256v1", "1.2.840.10045.3.1.7"),
];

impl Curve {
    pub fn name(self) -> &'static str {
        CURVES.iter().find(|c| c.0 == self).map_or("", |c| c.1)
    }

    pub fn oid(self) -> &'static str {
        CURVES.iter().find(|c| c.0 == self).map_or("", |c| c.2)
    }

    pub fn by_oid(oid: &str) -> Option<Curve> {
        CURVES.iter().find(|c| c.2 == oid).map(|c| c.0)
    }

    /// By OpenSSL's short name (and the NIST aliases).
    pub fn by_name(name: &str) -> Option<Curve> {
        let name = match name {
            "P-256" | "secp256r1" => "prime256v1",
            "P-384" => "secp384r1",
            "P-521" => "secp521r1",
            n => n,
        };
        CURVES.iter().find(|c| c.1 == name).map(|c| c.0)
    }

    pub fn bits(self) -> usize {
        match self {
            Curve::P256 | Curve::K256 => 256,
            Curve::P384 => 384,
            Curve::P521 => 521,
        }
    }

    /// The length of a scalar or coordinate.
    pub fn field_len(self) -> usize {
        self.bits().div_ceil(8)
    }
}

pub enum Key {
    Rsa { public: RsaPublicKey, private: Option<Box<RsaPrivateKey>> },
    /// `point`: the public point, uncompressed; `d`: the private scalar.
    Ec { curve: Curve, point: Vec<u8>, d: Option<Vec<u8>> },
    Ed25519 { public: [u8; 32], seed: Option<[u8; 32]> },
    X25519 { public: [u8; 32], secret: Option<[u8; 32]> },
}

/// What can go wrong reading or using a key, as OpenSSL queues it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// No key in a form we read ("DECODER routines::unsupported").
    Unsupported,
    /// An encrypted key with the wrong passphrase ("bad decrypt").
    BadDecrypt,
    DataTooLarge,
    DataTooSmall,
    Padding,
    Operation,
    KeyTooSmall,
}

impl KeyError {
    pub fn queued(self) -> &'static str {
        match self {
            KeyError::Unsupported => "error:1E08010C:DECODER routines::unsupported",
            KeyError::BadDecrypt => "error:1C800064:Provider routines::bad decrypt",
            KeyError::DataTooLarge => "error:0200006E:rsa routines::data too large for key size",
            KeyError::DataTooSmall => "error:02000084:rsa routines::data too small for key size",
            KeyError::Padding => "error:02000079:rsa routines::oaep decoding error",
            KeyError::Operation => "error:03000096:digital envelope routines::operation not supported for this keytype",
            KeyError::KeyTooSmall => "error:1C8000AB:Provider routines::key size too small",
        }
    }
}

type Res<T> = Result<T, KeyError>;

fn big(bytes: &[u8]) -> BigUint {
    BigUint::from_bytes_be(bytes)
}

/// An RSA public key as OpenSSL reads one: any odd or even modulus (php-src
/// has a test certificate with an even one), up to 16384 bits.
fn rsa_public(n: BigUint, e: BigUint) -> Res<RsaPublicKey> {
    if n.bits() > 16384 || n.bits() == 0 || e.bits() == 0 {
        return Err(KeyError::Unsupported);
    }
    Ok(RsaPublicKey::new_unchecked(n, e))
}

/// A number's big-endian bytes, `len` long (left-padded with zeros).
fn fixed(n: &BigUint, len: usize) -> Vec<u8> {
    let b = n.to_bytes_be();
    if b.len() >= len {
        return b[b.len() - len..].to_vec();
    }
    let mut out = vec![0u8; len - b.len()];
    out.extend_from_slice(&b);
    out
}

/// A number's big-endian bytes with no leading zeros (what BN_bn2bin gives).
pub fn minimal(bytes: &[u8]) -> Vec<u8> {
    let skip = bytes.iter().take_while(|b| **b == 0).count();
    bytes[skip..].to_vec()
}

// ---- curve arithmetic over the RustCrypto crates ----

macro_rules! with_curve {
    ($curve:expr, $c:ident => $body:expr) => {
        match $curve {
            Curve::P256 => {
                use p256 as $c;
                $body
            }
            Curve::P384 => {
                use p384 as $c;
                $body
            }
            Curve::P521 => {
                use p521 as $c;
                $body
            }
            Curve::K256 => {
                use k256 as $c;
                $body
            }
        }
    };
}

fn ec_public_from_private(curve: Curve, d: &[u8]) -> Option<Vec<u8>> {
    use elliptic_curve::sec1::ToEncodedPoint;
    with_curve!(curve, c => {
        let sk = c::SecretKey::from_slice(d).ok()?;
        Some(sk.public_key().to_encoded_point(false).as_bytes().to_vec())
    })
}

/// A public point (any SEC1 form) as the uncompressed form, if on the curve.
fn ec_uncompressed(curve: Curve, point: &[u8]) -> Option<Vec<u8>> {
    use elliptic_curve::sec1::ToEncodedPoint;
    with_curve!(curve, c => {
        let pk = c::PublicKey::from_sec1_bytes(point).ok()?;
        Some(pk.to_encoded_point(false).as_bytes().to_vec())
    })
}

fn ec_generate(curve: Curve) -> (Vec<u8>, Vec<u8>) {
    use elliptic_curve::sec1::ToEncodedPoint;
    with_curve!(curve, c => {
        let sk = c::SecretKey::random(&mut OsRng);
        let point = sk.public_key().to_encoded_point(false).as_bytes().to_vec();
        (sk.to_bytes().to_vec(), point)
    })
}

/// A hash shorter than the curve's order as the number it stands for, at
/// the order's length (what ECDSA's bits2int makes of it).
fn ecdsa_hash(curve: Curve, hash: &[u8]) -> Vec<u8> {
    let n = curve.field_len();
    if hash.len() >= n {
        return hash.to_vec();
    }
    let mut out = vec![0u8; n - hash.len()];
    out.extend_from_slice(hash);
    out
}

fn ecdsa_sign(curve: Curve, d: &[u8], hash: &[u8]) -> Option<Vec<u8>> {
    use ecdsa::signature::hazmat::RandomizedPrehashSigner;
    let hash = &ecdsa_hash(curve, hash)[..];
    with_curve!(curve, c => {
        let sk = c::ecdsa::SigningKey::from_slice(d).ok()?;
        let sig: c::ecdsa::Signature = sk.sign_prehash_with_rng(&mut OsRng, hash).ok()?;
        Some(sig.to_der().as_bytes().to_vec())
    })
}

/// 1: valid; 0: not; -1: not a signature at all.
fn ecdsa_verify(curve: Curve, point: &[u8], hash: &[u8], sig: &[u8]) -> i64 {
    use ecdsa::signature::hazmat::PrehashVerifier;
    let hash = &ecdsa_hash(curve, hash)[..];
    with_curve!(curve, c => {
        let Ok(vk) = c::ecdsa::VerifyingKey::from_sec1_bytes(point) else { return -1 };
        let Ok(sig) = c::ecdsa::Signature::from_der(sig) else { return -1 };
        i64::from(vk.verify_prehash(hash, &sig).is_ok())
    })
}

fn ecdh(curve: Curve, d: &[u8], peer: &[u8]) -> Option<Vec<u8>> {
    with_curve!(curve, c => {
        let sk = c::SecretKey::from_slice(d).ok()?;
        let pk = c::PublicKey::from_sec1_bytes(peer).ok()?;
        let shared = elliptic_curve::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine());
        Some(shared.raw_secret_bytes().to_vec())
    })
}

impl Key {
    pub fn keytype(&self) -> i64 {
        match self {
            Key::Rsa { .. } => KEYTYPE_RSA,
            Key::Ec { .. } => KEYTYPE_EC,
            Key::Ed25519 { .. } => KEYTYPE_ED25519,
            Key::X25519 { .. } => KEYTYPE_X25519,
        }
    }

    /// EVP_PKEY_bits
    pub fn bits(&self) -> usize {
        match self {
            Key::Rsa { public, .. } => public.n().bits(),
            Key::Ec { curve, .. } => curve.bits(),
            Key::Ed25519 { .. } => 256,
            Key::X25519 { .. } => 253,
        }
    }

    /// The key's public half.
    #[cfg(test)]
    pub fn public(&self) -> Key {
        match self {
            Key::Rsa { public, .. } => Key::Rsa { public: public.clone(), private: None },
            Key::Ec { curve, point, .. } => Key::Ec { curve: *curve, point: point.clone(), d: None },
            Key::Ed25519 { public, .. } => Key::Ed25519 { public: *public, seed: None },
            Key::X25519 { public, .. } => Key::X25519 { public: *public, secret: None },
        }
    }

    /// Whether both are the same key pair's halves (X509_check_private_key).
    pub fn same_public(&self, other: &Key) -> bool {
        self.spki_der() == other.spki_der()
    }

    // ---- encoding ----

    /// SubjectPublicKeyInfo, DER.
    pub fn spki_der(&self) -> Vec<u8> {
        match self {
            Key::Rsa { public, .. } => {
                let rsa = der::seq(&[&der::uint(&public.n().to_bytes_be()), &der::uint(&public.e().to_bytes_be())]);
                der::seq(&[&der::seq(&[&der::oid(OID_RSA), &der::null()]), &der::bit_string(&rsa)])
            }
            Key::Ec { curve, point, .. } => der::seq(&[&der::seq(&[&der::oid(OID_EC), &der::oid(curve.oid())]), &der::bit_string(point)]),
            Key::Ed25519 { public, .. } => der::seq(&[&der::seq(&[&der::oid(OID_ED25519)]), &der::bit_string(public)]),
            Key::X25519 { public, .. } => der::seq(&[&der::seq(&[&der::oid(OID_X25519)]), &der::bit_string(public)]),
        }
    }

    pub fn public_pem(&self) -> String {
        der::pem_encode("PUBLIC KEY", &self.spki_der())
    }

    /// PrivateKeyInfo (PKCS#8), DER; None for a public key.
    pub fn pkcs8_der(&self) -> Option<Vec<u8>> {
        let (alg, inner) = match self {
            Key::Rsa { public, private } => {
                let p = private.as_ref()?;
                let primes = p.primes();
                let n = |b: &BigUint| der::uint(&b.to_bytes_be());
                let zero = BigUint::from(0u8);
                let inner = der::seq(&[
                    &der::small_int(0),
                    &n(public.n()),
                    &n(public.e()),
                    &n(p.d()),
                    &n(primes.first().unwrap_or(&zero)),
                    &n(primes.get(1).unwrap_or(&zero)),
                    &n(p.dp().unwrap_or(&zero)),
                    &n(p.dq().unwrap_or(&zero)),
                    &n(&p.crt_coefficient().unwrap_or_default()),
                ]);
                (der::seq(&[&der::oid(OID_RSA), &der::null()]), inner)
            }
            Key::Ec { curve, point, d } => {
                let d = d.as_ref()?;
                let public = der::tlv(0xa1, &der::bit_string(point));
                let inner = der::seq(&[&der::small_int(1), &der::octet_string(d), &public]);
                (der::seq(&[&der::oid(OID_EC), &der::oid(curve.oid())]), inner)
            }
            Key::Ed25519 { seed, .. } => (der::seq(&[&der::oid(OID_ED25519)]), der::octet_string(seed.as_ref()?)),
            Key::X25519 { secret, .. } => (der::seq(&[&der::oid(OID_X25519)]), der::octet_string(secret.as_ref()?)),
        };
        Some(der::seq(&[&der::small_int(0), &alg, &der::octet_string(&inner)]))
    }

    /// PEM_write_bio_PrivateKey: PKCS#8, encrypted (PBES2) under
    /// `cipher` when a passphrase is given.
    pub fn private_pem(&self, encrypt: Option<(&str, &[u8])>) -> Option<String> {
        let pkcs8 = self.pkcs8_der()?;
        match encrypt {
            None => Some(der::pem_encode("PRIVATE KEY", &pkcs8)),
            Some((cipher, pass)) => Some(der::pem_encode("ENCRYPTED PRIVATE KEY", &pbes2_encrypt(&pkcs8, cipher, pass)?)),
        }
    }

    // ---- reading ----

    /// A SubjectPublicKeyInfo.
    pub fn from_spki(spki: &[u8]) -> Res<Key> {
        let t = der::parse_one(spki).ok_or(KeyError::Unsupported)?;
        let mut r = t.reader();
        let alg = r.expect(der::SEQUENCE).ok_or(KeyError::Unsupported)?;
        let bits = r.expect(der::BIT_STRING).ok_or(KeyError::Unsupported)?;
        let key = bits.body.get(1..).ok_or(KeyError::Unsupported)?;
        let mut a = alg.reader();
        let oid = der::oid_text(a.expect(der::OID).ok_or(KeyError::Unsupported)?.body);
        match oid.as_str() {
            OID_RSA => {
                let s = der::parse_one(key).ok_or(KeyError::Unsupported)?;
                let mut s = s.reader();
                let n = s.expect(der::INTEGER).ok_or(KeyError::Unsupported)?;
                let e = s.expect(der::INTEGER).ok_or(KeyError::Unsupported)?;
                let public = rsa_public(big(n.body), big(e.body))?;
                Ok(Key::Rsa { public, private: None })
            }
            OID_EC => {
                let curve_oid = a.expect(der::OID).ok_or(KeyError::Unsupported)?;
                let curve = Curve::by_oid(&der::oid_text(curve_oid.body)).ok_or(KeyError::Unsupported)?;
                let point = ec_uncompressed(curve, key).ok_or(KeyError::Unsupported)?;
                Ok(Key::Ec { curve, point, d: None })
            }
            OID_ED25519 => Ok(Key::Ed25519 { public: key.try_into().map_err(|_| KeyError::Unsupported)?, seed: None }),
            OID_X25519 => Ok(Key::X25519 { public: key.try_into().map_err(|_| KeyError::Unsupported)?, secret: None }),
            _ => Err(KeyError::Unsupported),
        }
    }

    /// An RSAPrivateKey (PKCS#1).
    pub fn from_pkcs1(data: &[u8]) -> Res<Key> {
        let t = der::parse_one(data).ok_or(KeyError::Unsupported)?;
        let mut r = t.reader();
        let mut ints = Vec::new();
        while let Some(i) = r.optional(der::INTEGER) {
            ints.push(big(i.body));
        }
        if ints.len() < 9 {
            return Err(KeyError::Unsupported);
        }
        let (n, e, d, p, q) = (ints[1].clone(), ints[2].clone(), ints[3].clone(), ints[4].clone(), ints[5].clone());
        let mut private = RsaPrivateKey::from_components(n.clone(), e.clone(), d, vec![p, q]).map_err(|_| KeyError::Unsupported)?;
        private.precompute().map_err(|_| KeyError::Unsupported)?;
        let public = rsa_public(n, e)?;
        Ok(Key::Rsa { public, private: Some(Box::new(private)) })
    }

    /// An ECPrivateKey (SEC1), its curve from its own parameters or `curve`.
    pub fn from_sec1(data: &[u8], curve: Option<Curve>) -> Res<Key> {
        let t = der::parse_one(data).ok_or(KeyError::Unsupported)?;
        let mut r = t.reader();
        r.expect(der::INTEGER).ok_or(KeyError::Unsupported)?;
        let d = r.expect(der::OCTET_STRING).ok_or(KeyError::Unsupported)?.body.to_vec();
        let mut curve = curve;
        let mut point = None;
        while let Some(t) = r.read() {
            match t.tag {
                0xa0 => {
                    if let Some(o) = der::parse_one(t.body).filter(|o| o.tag == der::OID) {
                        curve = Curve::by_oid(&der::oid_text(o.body)).or(curve);
                    }
                }
                0xa1 => {
                    if let Some(b) = der::parse_one(t.body).filter(|b| b.tag == der::BIT_STRING) {
                        point = b.body.get(1..).map(<[u8]>::to_vec);
                    }
                }
                _ => {}
            }
        }
        let curve = curve.ok_or(KeyError::Unsupported)?;
        let mut d = d;
        if d.len() < curve.field_len() {
            let mut padded = vec![0u8; curve.field_len() - d.len()];
            padded.extend_from_slice(&d);
            d = padded;
        }
        let derived = ec_public_from_private(curve, &d).ok_or(KeyError::Unsupported)?;
        let point = match point.and_then(|p| ec_uncompressed(curve, &p)) {
            Some(p) => p,
            None => derived,
        };
        Ok(Key::Ec { curve, point, d: Some(d) })
    }

    /// A PrivateKeyInfo (PKCS#8).
    pub fn from_pkcs8(data: &[u8]) -> Res<Key> {
        let t = der::parse_one(data).ok_or(KeyError::Unsupported)?;
        let mut r = t.reader();
        r.expect(der::INTEGER).ok_or(KeyError::Unsupported)?;
        let alg = r.expect(der::SEQUENCE).ok_or(KeyError::Unsupported)?;
        let inner = r.expect(der::OCTET_STRING).ok_or(KeyError::Unsupported)?.body;
        let mut a = alg.reader();
        let oid = der::oid_text(a.expect(der::OID).ok_or(KeyError::Unsupported)?.body);
        match oid.as_str() {
            OID_RSA => Key::from_pkcs1(inner),
            OID_EC => {
                let curve = a.optional(der::OID).and_then(|o| Curve::by_oid(&der::oid_text(o.body)));
                Key::from_sec1(inner, curve)
            }
            OID_ED25519 => {
                let seed: [u8; 32] = der::parse_one(inner).filter(|o| o.tag == der::OCTET_STRING).ok_or(KeyError::Unsupported)?.body.try_into().map_err(|_| KeyError::Unsupported)?;
                Ok(Key::ed25519_from_seed(seed))
            }
            OID_X25519 => {
                let secret: [u8; 32] = der::parse_one(inner).filter(|o| o.tag == der::OCTET_STRING).ok_or(KeyError::Unsupported)?.body.try_into().map_err(|_| KeyError::Unsupported)?;
                Ok(Key::x25519_from_secret(secret))
            }
            _ => Err(KeyError::Unsupported),
        }
    }

    pub fn ed25519_from_seed(seed: [u8; 32]) -> Key {
        let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
        Key::Ed25519 { public: sk.verifying_key().to_bytes(), seed: Some(seed) }
    }

    pub fn x25519_from_secret(secret: [u8; 32]) -> Key {
        let s = x25519_dalek::StaticSecret::from(secret);
        Key::X25519 { public: x25519_dalek::PublicKey::from(&s).to_bytes(), secret: Some(secret) }
    }

    /// The first public key in PEM `text` (PEM_read_bio_PUBKEY).
    pub fn public_from_pem(text: &[u8]) -> Res<Key> {
        let pem = der::pem_find(text, &["PUBLIC KEY"]).ok_or(KeyError::Unsupported)?;
        Key::from_spki(&pem.der)
    }

    /// The first private key in PEM `text` (PEM_read_bio_PrivateKey),
    /// decrypted with `pass` when it is encrypted.
    pub fn private_from_pem(text: &[u8], pass: Option<&[u8]>) -> Res<Key> {
        let labels = ["PRIVATE KEY", "RSA PRIVATE KEY", "EC PRIVATE KEY", "ENCRYPTED PRIVATE KEY"];
        let pem = der::pem_find(text, &labels).ok_or(KeyError::Unsupported)?;
        let encrypted = pem.headers.iter().any(|(k, v)| k == "Proc-Type" && v.contains("ENCRYPTED"));
        let body = if encrypted {
            let info = pem.headers.iter().find(|(k, _)| k == "DEK-Info").map(|(_, v)| v.clone()).ok_or(KeyError::Unsupported)?;
            legacy_decrypt(&pem.der, &info, pass.unwrap_or_default())?
        } else {
            pem.der.clone()
        };
        match pem.label.as_str() {
            "PRIVATE KEY" => Key::from_pkcs8(&body),
            "RSA PRIVATE KEY" => Key::from_pkcs1(&body),
            "EC PRIVATE KEY" => Key::from_sec1(&body, None),
            _ => Key::from_pkcs8(&pbes2_decrypt(&body, pass.unwrap_or_default())?),
        }
    }

    // ---- generating ----

    pub fn generate_rsa(bits: usize, e: u64) -> Res<Key> {
        if bits < 512 {
            return Err(KeyError::KeyTooSmall);
        }
        let private = RsaPrivateKey::new_with_exp(&mut OsRng, bits, &BigUint::from(e)).map_err(|_| KeyError::Operation)?;
        let public = private.to_public_key();
        Ok(Key::Rsa { public, private: Some(Box::new(private)) })
    }

    pub fn generate_ec(curve: Curve) -> Key {
        let (d, point) = ec_generate(curve);
        Key::Ec { curve, point, d: Some(d) }
    }

    pub fn generate_ed25519() -> Key {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        Key::ed25519_from_seed(seed)
    }

    pub fn generate_x25519() -> Key {
        let mut secret = [0u8; 32];
        OsRng.fill_bytes(&mut secret);
        Key::x25519_from_secret(secret)
    }

    /// An RSA key from its parts (openssl_pkey_new's 'rsa' option).
    pub fn rsa_from_parts(n: &[u8], e: &[u8], d: Option<&[u8]>, p: Option<&[u8]>, q: Option<&[u8]>) -> Res<Key> {
        let public = rsa_public(big(n), big(e))?;
        let private = match d {
            Some(d) => {
                let primes = match (p, q) {
                    (Some(p), Some(q)) => vec![big(p), big(q)],
                    _ => Vec::new(),
                };
                let mut k = RsaPrivateKey::from_components(big(n), big(e), big(d), primes).map_err(|_| KeyError::Unsupported)?;
                k.precompute().map_err(|_| KeyError::Unsupported)?;
                Some(Box::new(k))
            }
            None => None,
        };
        Ok(Key::Rsa { public, private })
    }

    /// An EC key from its parts (openssl_pkey_new's 'ec' option).
    pub fn ec_from_parts(curve: Curve, d: Option<&[u8]>, x: Option<&[u8]>, y: Option<&[u8]>) -> Res<Key> {
        let n = curve.field_len();
        let pad = |b: &[u8]| fixed(&big(b), n);
        let d = d.map(pad);
        let point = match (x, y, &d) {
            (Some(x), Some(y), _) => {
                let mut p = vec![4u8];
                p.extend(pad(x));
                p.extend(pad(y));
                ec_uncompressed(curve, &p).ok_or(KeyError::Unsupported)?
            }
            (_, _, Some(d)) => ec_public_from_private(curve, d).ok_or(KeyError::Unsupported)?,
            _ => return Err(KeyError::Unsupported),
        };
        if let Some(d) = &d
            && ec_public_from_private(curve, d).as_deref() != Some(&point[..])
        {
            return Err(KeyError::Unsupported);
        }
        Ok(Key::Ec { curve, point, d })
    }

    // ---- what openssl_pkey_get_details() shows ----

    /// (name, minimal big-endian bytes) of the key's numbers.
    pub fn rsa_parts(&self) -> Vec<(&'static str, Vec<u8>)> {
        let Key::Rsa { public, private } = self else { return Vec::new() };
        let mut out = vec![("n", public.n().to_bytes_be()), ("e", public.e().to_bytes_be())];
        if let Some(p) = private {
            out.push(("d", p.d().to_bytes_be()));
            let primes = p.primes();
            if primes.len() >= 2 {
                out.push(("p", primes[0].to_bytes_be()));
                out.push(("q", primes[1].to_bytes_be()));
            }
            if let (Some(dp), Some(dq), Some(qi)) = (p.dp(), p.dq(), p.crt_coefficient()) {
                out.push(("dmp1", dp.to_bytes_be()));
                out.push(("dmq1", dq.to_bytes_be()));
                out.push(("iqmp", qi.to_bytes_be()));
            }
        }
        out
    }

    // ---- signatures ----

    /// openssl_sign: `md` None means the key type's default digest.
    pub fn sign(&self, md: Option<Md>, data: &[u8]) -> Res<Vec<u8>> {
        match self {
            Key::Rsa { private: Some(p), public } => {
                let md = md.unwrap_or(Md::Sha256);
                let em = pkcs1_type1(&digest_info(md, data), public.size())?;
                rsa_private_op(p, &em)
            }
            Key::Ec { curve, d: Some(d), .. } => {
                let hash = md.unwrap_or(Md::Sha256).hash(data);
                ecdsa_sign(*curve, d, &hash).ok_or(KeyError::Operation)
            }
            Key::Ed25519 { seed: Some(seed), .. } => {
                if md.is_some() {
                    return Err(KeyError::Operation);
                }
                use ed25519_dalek::Signer;
                Ok(ed25519_dalek::SigningKey::from_bytes(seed).sign(data).to_bytes().to_vec())
            }
            _ => Err(KeyError::Operation),
        }
    }

    /// openssl_verify: 1, 0, or -1 for an error.
    pub fn verify(&self, md: Option<Md>, data: &[u8], sig: &[u8]) -> i64 {
        match self {
            Key::Rsa { public, .. } => {
                let md = md.unwrap_or(Md::Sha256);
                let Ok(expected) = pkcs1_type1(&digest_info(md, data), public.size()) else { return -1 };
                if sig.len() != public.size() {
                    return 0;
                }
                match rsa_public_op(public, sig) {
                    Ok(em) => i64::from(em == expected),
                    Err(_) => 0,
                }
            }
            Key::Ec { curve, point, .. } => ecdsa_verify(*curve, point, &md.unwrap_or(Md::Sha256).hash(data), sig),
            Key::Ed25519 { public, .. } => {
                if md.is_some() {
                    return -1;
                }
                let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(public) else { return -1 };
                let Ok(sig) = ed25519_dalek::Signature::from_slice(sig) else { return 0 };
                i64::from(vk.verify_strict(data, &sig).is_ok())
            }
            Key::X25519 { .. } => -1,
        }
    }

    // ---- RSA encryption ----

    /// openssl_public_encrypt
    pub fn public_encrypt(&self, data: &[u8], padding: i64) -> Res<Vec<u8>> {
        let Key::Rsa { public, .. } = self else { return Err(KeyError::Operation) };
        let k = public.size();
        let em = match padding {
            PKCS1_PADDING => pkcs1_type2(data, k)?,
            OAEP_PADDING => oaep_pad(data, k)?,
            NO_PADDING => no_pad(data, k)?,
            _ => return Err(KeyError::Operation),
        };
        rsa_public_op(public, &em)
    }

    /// openssl_private_decrypt
    pub fn private_decrypt(&self, data: &[u8], padding: i64) -> Res<Vec<u8>> {
        let Key::Rsa { private: Some(p), public } = self else { return Err(KeyError::Operation) };
        let k = public.size();
        if data.len() > k {
            return Err(KeyError::DataTooLarge);
        }
        let em = rsa_private_op(p, data)?;
        match padding {
            PKCS1_PADDING => pkcs1_unpad(&em, 2),
            OAEP_PADDING => oaep_unpad(&em),
            NO_PADDING => Ok(em),
            _ => Err(KeyError::Operation),
        }
    }

    /// openssl_private_encrypt (a raw signature)
    pub fn private_encrypt(&self, data: &[u8], padding: i64) -> Res<Vec<u8>> {
        let Key::Rsa { private: Some(p), public } = self else { return Err(KeyError::Operation) };
        let k = public.size();
        let em = match padding {
            PKCS1_PADDING => pkcs1_type1(data, k)?,
            NO_PADDING => no_pad(data, k)?,
            _ => return Err(KeyError::Operation),
        };
        rsa_private_op(p, &em)
    }

    /// openssl_public_decrypt (recovering what a raw signature holds)
    pub fn public_decrypt(&self, data: &[u8], padding: i64) -> Res<Vec<u8>> {
        let Key::Rsa { public, .. } = self else { return Err(KeyError::Operation) };
        if data.len() != public.size() {
            return Err(if data.len() > public.size() { KeyError::DataTooLarge } else { KeyError::DataTooSmall });
        }
        let em = rsa_public_op(public, data)?;
        match padding {
            PKCS1_PADDING => pkcs1_unpad(&em, 1),
            NO_PADDING => Ok(em),
            _ => Err(KeyError::Operation),
        }
    }

    /// openssl_pkey_derive / openssl_dh_compute_key: the shared secret.
    pub fn derive(&self, peer: &Key) -> Res<Vec<u8>> {
        match (self, peer) {
            (Key::Ec { curve, d: Some(d), .. }, Key::Ec { curve: c2, point, .. }) if curve == c2 => ecdh(*curve, d, point).ok_or(KeyError::Operation),
            (Key::X25519 { secret: Some(s), .. }, Key::X25519 { public, .. }) => {
                let shared = x25519_dalek::StaticSecret::from(*s).diffie_hellman(&x25519_dalek::PublicKey::from(*public));
                Ok(shared.as_bytes().to_vec())
            }
            _ => Err(KeyError::Operation),
        }
    }
}

pub const PKCS1_PADDING: i64 = 1;
pub const NO_PADDING: i64 = 3;
pub const OAEP_PADDING: i64 = 4;

fn digest_info(md: Md, data: &[u8]) -> Vec<u8> {
    let hash = md.hash(data);
    match md.digest_info_prefix() {
        Some(mut prefix) => {
            prefix.extend_from_slice(&hash);
            prefix
        }
        None => hash,
    }
}

fn rsa_private_op(key: &RsaPrivateKey, em: &[u8]) -> Res<Vec<u8>> {
    let c = big(em);
    if &c >= key.n() {
        return Err(KeyError::DataTooLarge);
    }
    let m = rsa::hazmat::rsa_decrypt_and_check(key, Some(&mut OsRng), &c).map_err(|_| KeyError::Operation)?;
    Ok(fixed(&m, key.size()))
}

fn rsa_public_op(key: &RsaPublicKey, data: &[u8]) -> Res<Vec<u8>> {
    // OpenSSL's Montgomery exponentiation needs an odd modulus.
    if key.n().to_bytes_be().last().is_none_or(|b| b & 1 == 0) {
        return Err(KeyError::Operation);
    }
    let m = big(data);
    if &m >= key.n() {
        return Err(KeyError::DataTooLarge);
    }
    let c = rsa::hazmat::rsa_encrypt(key, &m).map_err(|_| KeyError::Operation)?;
    Ok(fixed(&c, key.size()))
}

fn pkcs1_type1(data: &[u8], k: usize) -> Res<Vec<u8>> {
    if data.len() + 11 > k {
        return Err(KeyError::DataTooLarge);
    }
    let mut em = vec![0u8, 1];
    em.extend(std::iter::repeat_n(0xff, k - 3 - data.len()));
    em.push(0);
    em.extend_from_slice(data);
    Ok(em)
}

fn pkcs1_type2(data: &[u8], k: usize) -> Res<Vec<u8>> {
    if data.len() + 11 > k {
        return Err(KeyError::DataTooLarge);
    }
    let mut em = vec![0u8, 2];
    for _ in 0..k - 3 - data.len() {
        let mut b = [0u8];
        while b[0] == 0 {
            OsRng.fill_bytes(&mut b);
        }
        em.push(b[0]);
    }
    em.push(0);
    em.extend_from_slice(data);
    Ok(em)
}

fn pkcs1_unpad(em: &[u8], kind: u8) -> Res<Vec<u8>> {
    if em.len() < 11 || em[0] != 0 || em[1] != kind {
        return Err(KeyError::Padding);
    }
    let sep = em[2..].iter().position(|b| *b == 0).ok_or(KeyError::Padding)? + 2;
    if sep < 10 || (kind == 1 && em[2..sep].iter().any(|b| *b != 0xff)) {
        return Err(KeyError::Padding);
    }
    Ok(em[sep + 1..].to_vec())
}

fn no_pad(data: &[u8], k: usize) -> Res<Vec<u8>> {
    match data.len().cmp(&k) {
        std::cmp::Ordering::Greater => Err(KeyError::DataTooLarge),
        std::cmp::Ordering::Less => Err(KeyError::DataTooSmall),
        std::cmp::Ordering::Equal => Ok(data.to_vec()),
    }
}

fn mgf1(seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 20);
    let mut counter = 0u32;
    while out.len() < len {
        out.extend(Md::Sha1.hash(&[seed, &counter.to_be_bytes()].concat()));
        counter += 1;
    }
    out.truncate(len);
    out
}

/// RSAES-OAEP with SHA-1 and an empty label (RSA_PKCS1_OAEP_PADDING).
fn oaep_pad(data: &[u8], k: usize) -> Res<Vec<u8>> {
    let h = 20;
    if data.len() + 2 * h + 2 > k {
        return Err(KeyError::DataTooLarge);
    }
    let mut db = Md::Sha1.hash(b"");
    db.extend(std::iter::repeat_n(0, k - data.len() - 2 * h - 2));
    db.push(1);
    db.extend_from_slice(data);
    let mut seed = vec![0u8; h];
    OsRng.fill_bytes(&mut seed);
    for (a, b) in db.iter_mut().zip(mgf1(&seed, k - h - 1)) {
        *a ^= b;
    }
    for (a, b) in seed.iter_mut().zip(mgf1(&db, h)) {
        *a ^= b;
    }
    let mut em = vec![0u8];
    em.extend(seed);
    em.extend(db);
    Ok(em)
}

fn oaep_unpad(em: &[u8]) -> Res<Vec<u8>> {
    let h = 20;
    if em.len() < 2 * h + 2 || em[0] != 0 {
        return Err(KeyError::Padding);
    }
    let mut seed = em[1..1 + h].to_vec();
    let mut db = em[1 + h..].to_vec();
    for (a, b) in seed.iter_mut().zip(mgf1(&db, h)) {
        *a ^= b;
    }
    for (a, b) in db.iter_mut().zip(mgf1(&seed, em.len() - h - 1)) {
        *a ^= b;
    }
    if db[..h] != Md::Sha1.hash(b"")[..] {
        return Err(KeyError::Padding);
    }
    let rest = &db[h..];
    let one = rest.iter().position(|b| *b != 0).ok_or(KeyError::Padding)?;
    if rest[one] != 1 {
        return Err(KeyError::Padding);
    }
    Ok(rest[one + 1..].to_vec())
}

// ---- encrypted keys ----

/// EVP_BytesToKey with MD5 and one round: how legacy PEM encryption
/// turns a passphrase into a key.
pub fn bytes_to_key(pass: &[u8], salt: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut prev: Vec<u8> = Vec::new();
    while out.len() < len {
        prev = Md::Md5.hash(&[&prev[..], pass, salt].concat());
        out.extend_from_slice(&prev);
    }
    out.truncate(len);
    out
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn legacy_decrypt(data: &[u8], dek_info: &str, pass: &[u8]) -> Res<Vec<u8>> {
    let (name, iv) = dek_info.split_once(',').ok_or(KeyError::Unsupported)?;
    let iv = unhex(iv.trim()).ok_or(KeyError::Unsupported)?;
    let spec = crate::cipher::by_name(name.trim().as_bytes()).ok_or(KeyError::Unsupported)?;
    let key = bytes_to_key(pass, &iv[..8.min(iv.len())], spec.key_len);
    crate::cipher::simple(spec.name, &key, &iv, data, false).ok_or(KeyError::BadDecrypt)
}

const OID_PBES2: &str = "1.2.840.113549.1.5.13";
const OID_PBKDF2: &str = "1.2.840.113549.1.5.12";

fn prf_md(oid: &str) -> Option<Md> {
    Some(match oid {
        "1.2.840.113549.2.7" => Md::Sha1,
        "1.2.840.113549.2.8" => Md::Sha224,
        "1.2.840.113549.2.9" => Md::Sha256,
        "1.2.840.113549.2.10" => Md::Sha384,
        "1.2.840.113549.2.11" => Md::Sha512,
        _ => return None,
    })
}

fn pbes2_cipher(oid: &str) -> Option<&'static str> {
    Some(match oid {
        "2.16.840.1.101.3.4.1.2" => "aes-128-cbc",
        "2.16.840.1.101.3.4.1.22" => "aes-192-cbc",
        "2.16.840.1.101.3.4.1.42" => "aes-256-cbc",
        "1.2.840.113549.3.7" => "des-ede3-cbc",
        _ => return None,
    })
}

fn pbes2_cipher_oid(name: &str) -> Option<&'static str> {
    Some(match name {
        "aes-128-cbc" => "2.16.840.1.101.3.4.1.2",
        "aes-192-cbc" => "2.16.840.1.101.3.4.1.22",
        "aes-256-cbc" => "2.16.840.1.101.3.4.1.42",
        "des-ede3-cbc" => "1.2.840.113549.3.7",
        _ => return None,
    })
}

/// An EncryptedPrivateKeyInfo's PKCS#8 contents.
fn pbes2_decrypt(data: &[u8], pass: &[u8]) -> Res<Vec<u8>> {
    let bad = KeyError::Unsupported;
    let t = der::parse_one(data).ok_or(bad)?;
    let mut r = t.reader();
    let alg = r.expect(der::SEQUENCE).ok_or(bad)?;
    let enc = r.expect(der::OCTET_STRING).ok_or(bad)?.body;
    let mut a = alg.reader();
    if der::oid_text(a.expect(der::OID).ok_or(bad)?.body) != OID_PBES2 {
        return Err(bad);
    }
    let params = a.expect(der::SEQUENCE).ok_or(bad)?;
    let mut p = params.reader();
    let kdf = p.expect(der::SEQUENCE).ok_or(bad)?;
    let scheme = p.expect(der::SEQUENCE).ok_or(bad)?;
    let mut k = kdf.reader();
    if der::oid_text(k.expect(der::OID).ok_or(bad)?.body) != OID_PBKDF2 {
        return Err(bad);
    }
    let kp = k.expect(der::SEQUENCE).ok_or(bad)?;
    let mut kp = kp.reader();
    let salt = kp.expect(der::OCTET_STRING).ok_or(bad)?.body;
    let iter = kp.expect(der::INTEGER).ok_or(bad)?.body.iter().fold(0u64, |a, b| (a << 8) | u64::from(*b));
    let mut md = Md::Sha1;
    while let Some(t) = kp.read() {
        if t.tag == der::SEQUENCE {
            let mut pr = t.reader();
            md = prf_md(&der::oid_text(pr.expect(der::OID).ok_or(bad)?.body)).ok_or(bad)?;
        }
    }
    let mut s = scheme.reader();
    let cipher = pbes2_cipher(&der::oid_text(s.expect(der::OID).ok_or(bad)?.body)).ok_or(bad)?;
    let iv = s.expect(der::OCTET_STRING).ok_or(bad)?.body;
    let spec = crate::cipher::by_name(cipher.as_bytes()).ok_or(bad)?;
    let key = md.pbkdf2(pass, salt, u32::try_from(iter).map_err(|_| bad)?, spec.key_len);
    crate::cipher::simple(cipher, &key, iv, enc, false).ok_or(KeyError::BadDecrypt)
}

/// PKCS#8 encrypted as OpenSSL 3 does: PBES2, PBKDF2 with HMAC-SHA256,
/// 2048 rounds, a 16-byte salt.
fn pbes2_encrypt(pkcs8: &[u8], cipher: &str, pass: &[u8]) -> Option<Vec<u8>> {
    let spec = crate::cipher::by_name(cipher.as_bytes())?;
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let mut iv = vec![0u8; spec.iv_len];
    OsRng.fill_bytes(&mut iv);
    let key = Md::Sha256.pbkdf2(pass, &salt, 2048, spec.key_len);
    let enc = crate::cipher::simple(spec.name, &key, &iv, pkcs8, true)?;
    let prf = der::seq(&[&der::oid("1.2.840.113549.2.9"), &der::null()]);
    let kdf = der::seq(&[&der::oid(OID_PBKDF2), &der::seq(&[&der::octet_string(&salt), &der::small_int(2048), &prf])]);
    let scheme = der::seq(&[&der::oid(pbes2_cipher_oid(spec.name)?), &der::octet_string(&iv)]);
    let alg = der::seq(&[&der::oid(OID_PBES2), &der::seq(&[&kdf, &scheme])]);
    Some(der::seq(&[&alg, &der::octet_string(&enc)]))
}

/// Random bytes from the operating system.
pub fn random(len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    OsRng.fill_bytes(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsa_sign_verify_and_encrypt_round_trip() {
        let key = Key::generate_rsa(1024, 65537).unwrap();
        let sig = key.sign(Some(Md::Sha256), b"data").unwrap();
        assert_eq!(key.public().verify(Some(Md::Sha256), b"data", &sig), 1);
        assert_eq!(key.public().verify(Some(Md::Sha256), b"other", &sig), 0);
        for padding in [PKCS1_PADDING, OAEP_PADDING] {
            let ct = key.public().public_encrypt(b"secret", padding).unwrap();
            assert_eq!(key.private_decrypt(&ct, padding).unwrap(), b"secret");
        }
        let pem = key.private_pem(Some(("des-ede3-cbc", b"pw"))).unwrap();
        let back = Key::private_from_pem(pem.as_bytes(), Some(b"pw")).unwrap();
        assert!(back.same_public(&key));
        assert!(Key::private_from_pem(pem.as_bytes(), Some(b"wrong")).is_err());
    }

    #[test]
    fn ec_keys_round_trip() {
        for (curve, ..) in CURVES {
            let key = Key::generate_ec(*curve);
            let sig = key.sign(Some(Md::Sha256), b"x").unwrap();
            assert_eq!(key.public().verify(Some(Md::Sha256), b"x", &sig), 1);
            let back = Key::from_pkcs8(&key.pkcs8_der().unwrap()).unwrap();
            assert!(back.same_public(&key));
        }
    }
}
