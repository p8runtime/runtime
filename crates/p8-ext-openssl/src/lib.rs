#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's openssl extension over pure-Rust cryptography (RustCrypto and
//! dalek): no OpenSSL, no C. What it does follows ext/openssl of PHP 8.4
//! built against OpenSSL 3.5, down to its warnings and the error queue
//! openssl_error_string() reads.
//!
//! Done: certificates (openssl_x509_read, _parse, _export, _export_to_file,
//! _fingerprint, _check_private_key, _verify, _checkpurpose, _free), keys
//! (openssl_pkey_new, _get_private, _get_public, _get_details, _export,
//! _export_to_file, _derive, _free; RSA, EC on P-256/P-384/P-521/secp256k1,
//! Ed25519, X25519; PKCS#8, encrypted PKCS#8, PKCS#1, SEC1 and legacy
//! encrypted PEM), signatures (openssl_sign, openssl_verify), RSA
//! encryption (public/private encrypt/decrypt with PKCS#1, OAEP and no
//! padding), openssl_seal/openssl_open, symmetric ciphers (openssl_encrypt,
//! openssl_decrypt: AES, Camellia, triple DES, ChaCha20 in ECB, CBC, CTR,
//! CFB, CFB8, CFB1, OFB, GCM, CCM, ChaCha20-Poly1305), digests
//! (openssl_digest, openssl_pbkdf2), openssl_random_pseudo_bytes,
//! openssl_error_string, and the method and curve lists.
//!
//! Not implemented: CSRs (openssl_csr_*), SPKI (openssl_spki_*), PKCS#7,
//! PKCS#12 and CMS, DSA and DH keys, Ed448/X448, OCB mode, a text dump in
//! openssl_x509_export(..., false), the openssl.cnf configuration file, and
//! the TLS stream transports.
//!
//! Reference: $PHP_SRC_PATH/ext/openssl/openssl.c

mod cipher;
mod der;
mod digest;
mod oids;
mod pkey;
mod x509;

use p8_runtime::array::{Key as AKey, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{CLASS_FINAL, CLASS_NOT_SERIALIZABLE, CLASS_UNCLONEABLE, Native, Object};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use pkey::Key;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use x509::Cert;

pub const EXTENSION: Extension = Extension { name: "openssl", register };

type R = Result<Value, Throw>;

/// What OPENSSL_VERSION_TEXT and OPENSSL_VERSION_NUMBER report: the
/// OpenSSL release whose behaviour this follows.
const VERSION_TEXT: &[u8] = b"OpenSSL 3.5.0 8 Apr 2025";
const VERSION_NUMBER: i64 = 0x3050_0000;

fn register(registry: &mut Registry) {
    registry.add_functions(BUILTINS);
    registry.add_classes(CLASSES);
    registry.add_data("OpenSSLCertificate", || Rc::new(CertHandle::default()));
    registry.add_data("OpenSSLAsymmetricKey", || Rc::new(KeyHandle::default()));
    for (name, value) in INT_CONSTANTS {
        registry.add_constant(name.as_bytes(), Constant::Int(*value));
    }
    registry.add_constant(b"OPENSSL_VERSION_TEXT", Constant::Str(VERSION_TEXT));
    registry.add_constant(b"OPENSSL_VERSION_NUMBER", Constant::Int(VERSION_NUMBER));
    registry.add_constant(b"OPENSSL_DEFAULT_STREAM_CIPHERS", Constant::Str(DEFAULT_STREAM_CIPHERS));
    registry.add_ini("openssl.cafile", Some(b""));
    registry.add_ini("openssl.capath", Some(b""));
    registry.add_module("openssl");
}

const DEFAULT_STREAM_CIPHERS: &[u8] = b"ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-AES256-GCM-SHA384:DHE-RSA-AES128-GCM-SHA256:DHE-DSS-AES128-GCM-SHA256:kEDH+AESGCM:ECDHE-RSA-AES128-SHA256:ECDHE-ECDSA-AES128-SHA256:ECDHE-RSA-AES128-SHA:ECDHE-ECDSA-AES128-SHA:ECDHE-RSA-AES256-SHA384:ECDHE-ECDSA-AES256-SHA384:ECDHE-RSA-AES256-SHA:ECDHE-ECDSA-AES256-SHA:DHE-RSA-AES128-SHA256:DHE-RSA-AES128-SHA:DHE-DSS-AES128-SHA256:DHE-RSA-AES256-SHA256:DHE-DSS-AES256-SHA:DHE-RSA-AES256-SHA:AES128-GCM-SHA256:AES256-GCM-SHA384:AES128:AES256:HIGH:!SSLv2:!aNULL:!eNULL:!EXPORT:!DES:!MD5:!RC4:!ADH";

static INT_CONSTANTS: &[(&str, i64)] = &[
    ("X509_PURPOSE_SSL_CLIENT", 1),
    ("X509_PURPOSE_SSL_SERVER", 2),
    ("X509_PURPOSE_NS_SSL_SERVER", 3),
    ("X509_PURPOSE_SMIME_SIGN", 4),
    ("X509_PURPOSE_SMIME_ENCRYPT", 5),
    ("X509_PURPOSE_CRL_SIGN", 6),
    ("X509_PURPOSE_ANY", 7),
    ("X509_PURPOSE_OCSP_HELPER", 8),
    ("X509_PURPOSE_TIMESTAMP_SIGN", 9),
    ("OPENSSL_ALGO_SHA1", 1),
    ("OPENSSL_ALGO_MD5", 2),
    ("OPENSSL_ALGO_MD4", 3),
    ("OPENSSL_ALGO_SHA224", 6),
    ("OPENSSL_ALGO_SHA256", 7),
    ("OPENSSL_ALGO_SHA384", 8),
    ("OPENSSL_ALGO_SHA512", 9),
    ("OPENSSL_ALGO_RMD160", 10),
    ("PKCS7_DETACHED", 64),
    ("PKCS7_TEXT", 1),
    ("PKCS7_NOINTERN", 16),
    ("PKCS7_NOVERIFY", 32),
    ("PKCS7_NOCHAIN", 8),
    ("PKCS7_NOCERTS", 2),
    ("PKCS7_NOATTR", 256),
    ("PKCS7_BINARY", 128),
    ("PKCS7_NOSIGS", 4),
    ("PKCS7_NOOLDMIMETYPE", 1024),
    ("OPENSSL_CMS_DETACHED", 64),
    ("OPENSSL_CMS_TEXT", 1),
    ("OPENSSL_CMS_NOINTERN", 16),
    ("OPENSSL_CMS_NOVERIFY", 32),
    ("OPENSSL_CMS_NOCERTS", 2),
    ("OPENSSL_CMS_NOATTR", 256),
    ("OPENSSL_CMS_BINARY", 128),
    ("OPENSSL_CMS_NOSIGS", 12),
    ("OPENSSL_CMS_OLDMIMETYPE", 1024),
    ("OPENSSL_PKCS1_PADDING", 1),
    ("OPENSSL_NO_PADDING", 3),
    ("OPENSSL_PKCS1_OAEP_PADDING", 4),
    ("OPENSSL_CIPHER_RC2_40", 0),
    ("OPENSSL_CIPHER_RC2_128", 1),
    ("OPENSSL_CIPHER_RC2_64", 2),
    ("OPENSSL_CIPHER_DES", 3),
    ("OPENSSL_CIPHER_3DES", 4),
    ("OPENSSL_CIPHER_AES_128_CBC", 5),
    ("OPENSSL_CIPHER_AES_192_CBC", 6),
    ("OPENSSL_CIPHER_AES_256_CBC", 7),
    ("OPENSSL_KEYTYPE_RSA", 0),
    ("OPENSSL_KEYTYPE_DSA", 1),
    ("OPENSSL_KEYTYPE_DH", 2),
    ("OPENSSL_KEYTYPE_EC", 3),
    ("OPENSSL_KEYTYPE_X25519", 4),
    ("OPENSSL_KEYTYPE_ED25519", 5),
    ("OPENSSL_KEYTYPE_X448", 6),
    ("OPENSSL_KEYTYPE_ED448", 7),
    ("OPENSSL_RAW_DATA", 1),
    ("OPENSSL_ZERO_PADDING", 2),
    ("OPENSSL_DONT_ZERO_PAD_KEY", 4),
    ("OPENSSL_TLSEXT_SERVER_NAME", 1),
    ("OPENSSL_ENCODING_DER", 0),
    ("OPENSSL_ENCODING_SMIME", 1),
    ("OPENSSL_ENCODING_PEM", 2),
];

macro_rules! b {
    ($name:literal, $f:ident) => {
        Builtin { name: $name, run: $f, by_ref: &[] }
    };
    ($name:literal, $f:ident, $($r:literal),+) => {
        Builtin { name: $name, run: $f, by_ref: &[$($r),+] }
    };
}

static BUILTINS: &[Builtin] = &[
    b!("openssl_get_cert_locations", openssl_get_cert_locations),
    b!("openssl_x509_read", openssl_x509_read),
    b!("openssl_x509_free", openssl_x509_free),
    b!("openssl_x509_parse", openssl_x509_parse),
    b!("openssl_x509_export", openssl_x509_export, 1),
    b!("openssl_x509_export_to_file", openssl_x509_export_to_file),
    b!("openssl_x509_fingerprint", openssl_x509_fingerprint),
    b!("openssl_x509_check_private_key", openssl_x509_check_private_key),
    b!("openssl_x509_verify", openssl_x509_verify),
    b!("openssl_x509_checkpurpose", openssl_x509_checkpurpose),
    b!("openssl_pkey_new", openssl_pkey_new),
    b!("openssl_pkey_export", openssl_pkey_export, 1),
    b!("openssl_pkey_export_to_file", openssl_pkey_export_to_file),
    b!("openssl_pkey_get_private", openssl_pkey_get_private),
    b!("openssl_get_privatekey", openssl_pkey_get_private),
    b!("openssl_pkey_get_public", openssl_pkey_get_public),
    b!("openssl_get_publickey", openssl_pkey_get_public),
    b!("openssl_pkey_free", openssl_pkey_free),
    b!("openssl_free_key", openssl_pkey_free),
    b!("openssl_pkey_get_details", openssl_pkey_get_details),
    b!("openssl_pkey_derive", openssl_pkey_derive),
    b!("openssl_sign", openssl_sign, 1),
    b!("openssl_verify", openssl_verify),
    b!("openssl_public_encrypt", openssl_public_encrypt, 1),
    b!("openssl_private_decrypt", openssl_private_decrypt, 1),
    b!("openssl_private_encrypt", openssl_private_encrypt, 1),
    b!("openssl_public_decrypt", openssl_public_decrypt, 1),
    b!("openssl_seal", openssl_seal, 1, 2, 5),
    b!("openssl_open", openssl_open, 1),
    b!("openssl_encrypt", openssl_encrypt, 5),
    b!("openssl_decrypt", openssl_decrypt),
    b!("openssl_cipher_iv_length", openssl_cipher_iv_length),
    b!("openssl_cipher_key_length", openssl_cipher_key_length),
    b!("openssl_digest", openssl_digest),
    b!("openssl_pbkdf2", openssl_pbkdf2),
    b!("openssl_get_md_methods", openssl_get_md_methods),
    b!("openssl_get_cipher_methods", openssl_get_cipher_methods),
    b!("openssl_get_curve_names", openssl_get_curve_names),
    b!("openssl_random_pseudo_bytes", openssl_random_pseudo_bytes, 1),
    b!("openssl_error_string", openssl_error_string),
];

static CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "OpenSSLCertificate",
        parent: None,
        interfaces: &[],
        flags: CLASS_FINAL | CLASS_NOT_SERIALIZABLE | CLASS_UNCLONEABLE,
        properties: &[],
        methods: &[("__construct", Some(Native::Method(cert_construct)))],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "OpenSSLCertificateSigningRequest",
        parent: None,
        interfaces: &[],
        flags: CLASS_FINAL | CLASS_NOT_SERIALIZABLE | CLASS_UNCLONEABLE,
        properties: &[],
        methods: &[("__construct", Some(Native::Method(csr_construct)))],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "OpenSSLAsymmetricKey",
        parent: None,
        interfaces: &[],
        flags: CLASS_FINAL | CLASS_NOT_SERIALIZABLE | CLASS_UNCLONEABLE,
        properties: &[],
        methods: &[("__construct", Some(Native::Method(key_construct)))],
        constants: &[],
        debug_info: None,
    },
];

fn cert_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "Cannot directly construct OpenSSLCertificate, use openssl_x509_read() instead".into()))
}

fn csr_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "Cannot directly construct OpenSSLCertificateSigningRequest, use openssl_csr_new() instead".into()))
}

fn key_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "Cannot directly construct OpenSSLAsymmetricKey, use openssl_pkey_new() instead".into()))
}

// ---- helpers ----

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::NULL)
}

fn given(args: &[Value], i: usize) -> bool {
    args.get(i).map(Value::deref).is_some_and(|v| !v.is_null_or_undef())
}

fn bytes(args: &[Value], i: usize) -> Vec<u8> {
    arg(args, i).to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default()
}

fn opt_bytes(args: &[Value], i: usize) -> Option<Vec<u8>> {
    given(args, i).then(|| bytes(args, i))
}

fn set_ref(args: &[Value], i: usize, v: Value) {
    if let Some(cell) = args.get(i).and_then(Value::as_reference) {
        *cell.borrow_mut() = v;
    }
}

fn s(b: impl Into<Vec<u8>>) -> Value {
    Value::str(PhpStr::new(b.into()))
}

fn warn(ctx: &mut dyn Ctx, message: &str) {
    let name = ctx.builtin_name();
    ctx.warn(&format!("{name}(): {message}"));
}

fn value_error(ctx: &mut dyn Ctx, message: String) -> Throw {
    let name = ctx.builtin_name();
    ctx.throw("ValueError", format!("{name}(): {message}"))
}

fn put(a: &mut PhpArray, key: &str, v: Value) {
    a.insert(AKey::from_string(PhpStr::from(key)), v);
}

fn hex(b: &[u8]) -> Vec<u8> {
    der::hex_lower(b).into_bytes()
}

// ---- the error queue (PHP keeps the last 16) ----

/// What OpenSSL 3 queues for an algorithm no loaded provider offers.
const UNSUPPORTED: &str = "error:0308010C:digital envelope routines::unsupported";

struct Errors {
    queue: VecDeque<&'static str>,
    mark: Option<VecDeque<&'static str>>,
}

fn errors(ctx: &mut dyn Ctx) -> &mut Errors {
    let slot = ctx.ext_data("openssl.errors");
    if slot.as_ref().is_none_or(|b| !b.is::<Errors>()) {
        *slot = Some(Box::new(Errors { queue: VecDeque::new(), mark: None }));
    }
    slot.as_mut().and_then(|b| b.downcast_mut::<Errors>()).expect("the error queue")
}

fn push_error(ctx: &mut dyn Ctx, e: &'static str) {
    let q = &mut errors(ctx).queue;
    if q.len() == 15 {
        q.pop_front();
    }
    q.push_back(e);
}

fn set_mark(ctx: &mut dyn Ctx) {
    let e = errors(ctx);
    e.mark = Some(e.queue.clone());
}

fn restore_mark(ctx: &mut dyn Ctx) {
    let e = errors(ctx);
    e.queue = e.mark.take().unwrap_or_default();
}

fn openssl_error_string(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    match errors(ctx).queue.pop_front() {
        Some(e) => Ok(s(e)),
        None => Ok(Value::FALSE),
    }
}

// ---- reading files ----

/// The contents of a "file://" path, or the string itself; Err when the
/// file cannot be read (its errors queued).
fn data_or_file(ctx: &mut dyn Ctx, v: &[u8]) -> Option<Vec<u8>> {
    if v.len() > 7 && v.starts_with(b"file://") {
        let path = String::from_utf8_lossy(&v[7..]).into_owned();
        match std::fs::read(p8_runtime::cwd::resolve(ctx, &path)) {
            Ok(d) => Some(d),
            Err(_) => {
                push_error(ctx, "error:80000002:system library::No such file or directory");
                push_error(ctx, "error:10000080:BIO routines::no such file");
                None
            }
        }
    } else {
        Some(v.to_vec())
    }
}

/// php_openssl_check_path for a file written: the absolute path, or None
/// when it is not one (PHP warns, naming the parameter).
fn output_path(ctx: &mut dyn Ctx, p: &[u8], param: &str) -> Option<std::path::PathBuf> {
    let full = p8_runtime::cwd::resolve(ctx, String::from_utf8_lossy(p).as_ref());
    // expand_filepath fails past MAXPATHLEN
    if full.as_os_str().len() >= 4096 {
        let name = ctx.builtin_name();
        ctx.warn(&format!("{name}({param}): must be a valid file path"));
        return None;
    }
    Some(full)
}

// ---- certificates ----

/// An OpenSSLCertificate's state: set once, when openssl_x509_read makes it.
#[derive(Default)]
struct CertHandle(RefCell<Option<Rc<Cert>>>);

fn cert_of(o: &Object) -> Option<Rc<Cert>> {
    o.native::<CertHandle>()?.0.borrow().clone()
}

fn new_cert(ctx: &mut dyn Ctx, cert: Rc<Cert>) -> Value {
    let class = ctx.class(b"OpenSSLCertificate").expect("OpenSSLCertificate");
    let o = ctx.new_object(&class);
    if let Some(h) = o.native::<CertHandle>() {
        *h.0.borrow_mut() = Some(cert);
    }
    Value::object(o)
}

/// php_openssl_x509_from_str
fn cert_from_bytes(ctx: &mut dyn Ctx, v: &[u8]) -> Option<Rc<Cert>> {
    let data = data_or_file(ctx, v)?;
    match Cert::from_pem(&data) {
        Some(c) => Some(Rc::new(c)),
        None => {
            push_error(ctx, "error:0480006C:PEM routines::no start line");
            None
        }
    }
}

/// php_openssl_x509_from_zval: an OpenSSLCertificate, or a string (PEM, or file://).
fn cert_from_value(ctx: &mut dyn Ctx, v: &Value) -> Option<Rc<Cert>> {
    if let Some(o) = v.as_object()
        && let Some(c) = cert_of(o)
    {
        return Some(c);
    }
    let b = v.to_php_string()?;
    cert_from_bytes(ctx, b.as_bytes())
}

fn openssl_x509_read(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let v = arg(args, 0);
    match cert_from_value(ctx, &v) {
        Some(c) => Ok(new_cert(ctx, c)),
        None => {
            warn(ctx, "X.509 Certificate cannot be retrieved");
            Ok(Value::FALSE)
        }
    }
}

fn openssl_x509_free(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::NULL)
}

fn openssl_x509_export(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(cert) = cert_from_value(ctx, &arg(args, 0)) else {
        warn(ctx, "X.509 Certificate cannot be retrieved");
        return Ok(Value::FALSE);
    };
    set_ref(args, 1, s(cert.pem()));
    Ok(Value::TRUE)
}

fn openssl_x509_export_to_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(cert) = cert_from_value(ctx, &arg(args, 0)) else {
        warn(ctx, "X.509 Certificate cannot be retrieved");
        return Ok(Value::FALSE);
    };
    let Some(path) = output_path(ctx, &bytes(args, 1), "output_filename") else { return Ok(Value::FALSE) };
    if std::fs::write(&path, cert.pem()).is_err() {
        push_error(ctx, "error:80000002:system library::No such file or directory");
        warn(ctx, &format!("Error opening file {}", path.display()));
        return Ok(Value::FALSE);
    }
    Ok(Value::TRUE)
}

fn openssl_x509_fingerprint(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(cert) = cert_from_value(ctx, &arg(args, 0)) else {
        warn(ctx, "X.509 Certificate cannot be retrieved");
        return Ok(Value::FALSE);
    };
    let method = if given(args, 1) { bytes(args, 1) } else { b"sha1".to_vec() };
    let Some(md) = digest::by_name(&method) else {
        warn(ctx, "Unknown digest algorithm");
        return Ok(Value::FALSE);
    };
    let h = md.hash(&cert.der);
    Ok(s(if arg(args, 2).to_bool() { h } else { hex(&h) }))
}

fn openssl_x509_check_private_key(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(cert) = cert_from_value(ctx, &arg(args, 0)) else { return Ok(Value::FALSE) };
    let Some(key) = key_from_value(ctx, &arg(args, 1), false, Some(Vec::new()))? else { return Ok(Value::FALSE) };
    Ok(Value::bool(cert.public_key().is_some_and(|p| p.same_public(&key))))
}

fn openssl_x509_verify(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(cert) = cert_from_value(ctx, &arg(args, 0)) else { return Ok(Value::int(-1)) };
    let Some(key) = key_from_value(ctx, &arg(args, 1), true, None)? else { return Ok(Value::int(-1)) };
    Ok(Value::int(cert.verify(&key)))
}

/// openssl_x509_checkpurpose: the purpose checked on the certificate, and
/// its chain verified against the CA files and directories given.
fn openssl_x509_checkpurpose(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(cert) = cert_from_value(ctx, &arg(args, 0)) else { return Ok(Value::int(-1)) };
    let purpose = arg(args, 1).to_int();
    let mut trusted: Vec<Cert> = Vec::new();
    if let Ok(list) = arg(args, 2).into_array() {
        for v in list.values() {
            let path = p8_runtime::cwd::resolve(ctx, String::from_utf8_lossy(v.deref().to_php_string().unwrap_or_default().as_bytes()).as_ref());
            let files: Vec<std::path::PathBuf> = if path.is_dir() {
                std::fs::read_dir(&path).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
            } else {
                vec![path]
            };
            for f in files {
                if let Ok(text) = std::fs::read(&f) {
                    trusted.extend(der::pem_blocks(&text).into_iter().filter(|p| p.label == "CERTIFICATE").filter_map(|p| Cert::from_der(&p.der)));
                }
            }
        }
    }
    let mut untrusted: Vec<Cert> = Vec::new();
    if given(args, 3) {
        let path = String::from_utf8_lossy(&bytes(args, 3)).into_owned();
        if let Ok(text) = std::fs::read(p8_runtime::cwd::resolve(ctx, &path)) {
            untrusted.extend(der::pem_blocks(&text).into_iter().filter(|p| p.label == "CERTIFICATE").filter_map(|p| Cert::from_der(&p.der)));
        }
    }
    if !(1..=10).contains(&purpose) {
        return Ok(Value::FALSE);
    }
    if cert.check_purpose(purpose) != Some(true) {
        return Ok(Value::FALSE);
    }
    // Walk up to a trusted certificate, each link's signature and dates checked.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let in_date = |c: &Cert| {
        let from = x509::time_t(c.not_before.0, &c.not_before.1).unwrap_or(i64::MAX);
        let to = x509::time_t(c.not_after.0, &c.not_after.1).unwrap_or(i64::MIN);
        from <= now && now <= to
    };
    let mut current: &Cert = &cert;
    for _ in 0..10 {
        if !in_date(current) {
            return Ok(Value::FALSE);
        }
        if let Some(root) = trusted.iter().find(|t| t.der == current.der) {
            return Ok(Value::bool(root.purposes().iter().any(|p| p.0 == purpose)));
        }
        let issuer = trusted.iter().chain(untrusted.iter()).find(|c| c.subject.same(&current.issuer) && c.public_key().is_some_and(|k| current.verify(&k) == 1));
        match issuer {
            Some(i) => {
                if std::ptr::eq(i, current) {
                    return Ok(Value::FALSE);
                }
                if trusted.iter().any(|t| t.der == i.der) {
                    return Ok(Value::TRUE);
                }
                current = i;
            }
            None => return Ok(Value::FALSE),
        }
    }
    Ok(Value::FALSE)
}

fn openssl_x509_parse(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(cert) = cert_from_value(ctx, &arg(args, 0)) else { return Ok(Value::FALSE) };
    let short = !given(args, 1) || arg(args, 1).to_bool();
    let mut a = PhpArray::new();
    put(&mut a, "name", s(cert.subject.oneline()));
    put(&mut a, "subject", name_array(&cert.subject, short));
    put(&mut a, "hash", s(format!("{:08x}", cert.subject.hash())));
    put(&mut a, "issuer", name_array(&cert.issuer, short));
    put(&mut a, "version", Value::int(cert.version));
    let (dec, hexs) = x509::serial_strings(&cert.serial);
    put(&mut a, "serialNumber", s(dec));
    put(&mut a, "serialNumberHex", s(hexs));
    put(&mut a, "validFrom", s(cert.not_before.1.clone()));
    put(&mut a, "validTo", s(cert.not_after.1.clone()));
    for (key, (tag, t)) in [("validFrom_time_t", &cert.not_before), ("validTo_time_t", &cert.not_after)] {
        let n = match x509::time_t(*tag, t) {
            Ok(n) => n,
            Err(message) => {
                warn(ctx, &message);
                -1
            }
        };
        put(&mut a, key, Value::int(n));
    }
    let sig = oids::by_oid(&cert.sig_oid);
    put(&mut a, "signatureTypeSN", s(sig.map_or("UNDEF", |o| o.sn)));
    put(&mut a, "signatureTypeLN", s(sig.map_or("undefined", |o| o.ln)));
    put(&mut a, "signatureTypeNID", Value::int(sig.map_or(0, |o| o.nid)));
    let mut purposes = PhpArray::new();
    for (id, sn, ln, leaf, ca) in cert.purposes() {
        let item = PhpArray::from_list(vec![Value::bool(leaf), Value::bool(ca), s(if short { sn } else { ln })]);
        purposes.insert(AKey::Int(id), Value::array(item));
    }
    put(&mut a, "purposes", Value::array(purposes));
    let mut exts = PhpArray::new();
    for e in &cert.exts {
        let name = x509::ext_name(&e.oid);
        let shown = if e.oid == "2.5.29.17" {
            match x509::subject_alt_name(&e.value) {
                Some(v) => v,
                None => return Ok(Value::FALSE),
            }
        } else {
            x509::print_ext(e).unwrap_or_else(|| e.value.clone())
        };
        exts.insert(AKey::from_string(PhpStr::from(name)), s(shown));
    }
    put(&mut a, "extensions", Value::array(exts));
    Ok(Value::array(a))
}

/// php_openssl_add_assoc_name_entry: a repeated attribute becomes a list.
fn name_array(name: &x509::Name, short: bool) -> Value {
    let mut a = PhpArray::new();
    for (key, value) in name.pairs(short) {
        let k = AKey::from_string(PhpStr::from(key));
        match a.get(&k).cloned().map(Value::into_array) {
            Some(Ok(list)) => {
                let mut list = (*list).clone();
                let _ = list.push(s(value));
                a.insert(k, Value::array(list));
            }
            Some(Err(first)) => {
                let list = PhpArray::from_list(vec![first, s(value)]);
                a.insert(k, Value::array(list));
            }
            None => a.insert(k, s(value)),
        }
    }
    Value::array(a)
}

fn openssl_get_cert_locations(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    let mut a = PhpArray::new();
    put(&mut a, "default_cert_file", s("/usr/local/ssl/cert.pem"));
    put(&mut a, "default_cert_file_env", s("SSL_CERT_FILE"));
    put(&mut a, "default_cert_dir", s("/usr/local/ssl/certs"));
    put(&mut a, "default_cert_dir_env", s("SSL_CERT_DIR"));
    put(&mut a, "default_private_dir", s("/usr/local/ssl/private"));
    put(&mut a, "default_default_cert_area", s("/usr/local/ssl"));
    put(&mut a, "ini_cafile", s(""));
    put(&mut a, "ini_capath", s(""));
    Ok(Value::array(a))
}

// ---- keys ----

/// An OpenSSLAsymmetricKey's state: the key, and whether PHP holds it as a
/// private key.
#[derive(Default)]
struct KeyHandle(RefCell<Option<(Rc<Key>, bool)>>);

fn new_key(ctx: &mut dyn Ctx, key: Rc<Key>, private: bool) -> Value {
    let class = ctx.class(b"OpenSSLAsymmetricKey").expect("OpenSSLAsymmetricKey");
    let o = ctx.new_object(&class);
    if let Some(h) = o.native::<KeyHandle>() {
        *h.0.borrow_mut() = Some((key, private));
    }
    Value::object(o)
}

fn key_handle(v: &Value) -> Option<(Rc<Key>, bool)> {
    v.as_object()?.native::<KeyHandle>()?.0.borrow().clone()
}

/// php_openssl_pkey_from_zval: a key object, a certificate (its public
/// key), `[key, passphrase]`, or a string (PEM, or file://).
fn key_from_value(ctx: &mut dyn Ctx, v: &Value, public: bool, pass: Option<Vec<u8>>) -> Result<Option<Rc<Key>>, Throw> {
    let mut pass = pass;
    let mut v = v.clone();
    if let Some(a) = v.as_array() {
        let Some(phrase) = a.get(&AKey::Int(1)) else {
            return Err(ctx.throw("ValueError", "Key array must be of the form array(0 => key, 1 => phrase)".into()));
        };
        pass = Some(phrase.deref().to_php_string().map(|p| p.as_bytes().to_vec()).unwrap_or_default());
        let Some(k) = a.get(&AKey::Int(0)) else {
            return Err(ctx.throw("ValueError", "Key array must be of the form array(0 => key, 1 => phrase)".into()));
        };
        v = k.deref();
    }
    if let Some((key, private)) = key_handle(&v) {
        if !public && !private {
            warn(ctx, "Supplied key param is a public key");
            return Ok(None);
        }
        if public && private {
            warn(ctx, "Don't know how to get public key from this private key");
            return Ok(None);
        }
        return Ok(Some(key));
    }
    if let Some(o) = v.as_object()
        && let Some(c) = cert_of(o)
    {
        if !public {
            return Ok(None);
        }
        return Ok(c.public_key().map(Rc::new));
    }
    if !(v.is_str() || v.is_object()) {
        return Ok(None);
    }
    let Some(text) = v.to_php_string() else { return Ok(None) };
    let text = text.as_bytes();
    if public {
        set_mark(ctx);
        if let Some(cert) = cert_from_bytes(ctx, text) {
            return Ok(cert.public_key().map(Rc::new));
        }
        restore_mark(ctx);
        let Some(data) = data_or_file(ctx, text) else { return Ok(None) };
        match Key::public_from_pem(&data) {
            Ok(k) => Ok(Some(Rc::new(k))),
            Err(e) => {
                push_error(ctx, e.queued());
                Ok(None)
            }
        }
    } else {
        let Some(data) = data_or_file(ctx, text) else { return Ok(None) };
        match Key::private_from_pem(&data, pass.as_deref()) {
            Ok(k) => Ok(Some(Rc::new(k))),
            Err(e) => {
                push_error(ctx, e.queued());
                Ok(None)
            }
        }
    }
}

fn openssl_pkey_get_public(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    match key_from_value(ctx, &arg(args, 0), true, None)? {
        Some(k) => Ok(new_key(ctx, k, false)),
        None => Ok(Value::FALSE),
    }
}

fn openssl_pkey_get_private(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let pass = if args.len() > 1 { opt_bytes(args, 1) } else { Some(Vec::new()) };
    match key_from_value(ctx, &arg(args, 0), false, pass)? {
        Some(k) => Ok(new_key(ctx, k, true)),
        None => Ok(Value::FALSE),
    }
}

fn openssl_pkey_free(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::NULL)
}

fn openssl_pkey_get_details(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let v = arg(args, 0);
    let Some((key, _)) = key_handle(&v) else { return Ok(Value::FALSE) };
    let mut a = PhpArray::new();
    put(&mut a, "bits", Value::int(key.bits() as i64));
    put(&mut a, "key", s(key.public_pem()));
    let mut parts = PhpArray::new();
    let section = match &*key {
        Key::Rsa { .. } => {
            for (n, b) in key.rsa_parts() {
                put(&mut parts, n, s(b));
            }
            "rsa"
        }
        Key::Ec { curve, point, d } => {
            put(&mut parts, "curve_name", s(curve.name()));
            put(&mut parts, "curve_oid", s(curve.oid()));
            let n = curve.field_len();
            put(&mut parts, "x", s(pkey::minimal(&point[1..1 + n])));
            put(&mut parts, "y", s(pkey::minimal(&point[1 + n..])));
            if let Some(d) = d {
                put(&mut parts, "d", s(pkey::minimal(d)));
            }
            "ec"
        }
        Key::Ed25519 { public, seed } => {
            if let Some(seed) = seed {
                put(&mut parts, "priv_key", s(seed.to_vec()));
            }
            put(&mut parts, "pub_key", s(public.to_vec()));
            "ed25519"
        }
        Key::X25519 { public, secret } => {
            if let Some(secret) = secret {
                put(&mut parts, "priv_key", s(secret.to_vec()));
            }
            put(&mut parts, "pub_key", s(public.to_vec()));
            "x25519"
        }
    };
    put(&mut a, section, Value::array(parts));
    put(&mut a, "type", Value::int(key.keytype()));
    Ok(Value::array(a))
}

/// A string option of openssl_pkey_new()'s and openssl_pkey_export()'s options.
fn option(args: &[Value], i: usize, name: &str) -> Option<Value> {
    arg(args, i).as_array()?.get(&AKey::from_string(PhpStr::from(name))).map(Value::deref)
}

fn bn(a: &PhpArray, name: &str) -> Option<Vec<u8>> {
    a.get(&AKey::from_string(PhpStr::from(name))).map(Value::deref)?.as_str().map(|s| s.as_bytes().to_vec())
}

fn openssl_pkey_new(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    if let Some(Ok(data)) = option(args, 0, "rsa").map(Value::into_array) {
        let (Some(n), Some(d)) = (bn(&data, "n"), bn(&data, "d")) else { return Ok(Value::FALSE) };
        let e = bn(&data, "e").unwrap_or_else(|| vec![1, 0, 1]);
        return match Key::rsa_from_parts(&n, &e, Some(&d), bn(&data, "p").as_deref(), bn(&data, "q").as_deref()) {
            Ok(k) => Ok(new_key(ctx, Rc::new(k), true)),
            Err(e) => {
                push_error(ctx, e.queued());
                Ok(Value::FALSE)
            }
        };
    }
    if let Some(Ok(data)) = option(args, 0, "ec").map(Value::into_array) {
        let curve = match data.get(&AKey::from_string(PhpStr::from("curve_name"))).map(Value::deref).and_then(|v| v.into_str().ok()) {
            Some(c) if !c.is_empty() => {
                let name = String::from_utf8_lossy(c.as_bytes()).into_owned();
                match pkey::Curve::by_name(&name) {
                    Some(c) => c,
                    None => {
                        warn(ctx, &format!("Unknown elliptic curve (short) name {name}"));
                        return Ok(Value::FALSE);
                    }
                }
            }
            _ => {
                warn(ctx, "Missing params: curve_name");
                return Ok(Value::FALSE);
            }
        };
        let (d, x, y) = (bn(&data, "d"), bn(&data, "x"), bn(&data, "y"));
        if d.is_none() && x.is_none() && y.is_none() {
            return Ok(new_key(ctx, Rc::new(Key::generate_ec(curve)), true));
        }
        let private = d.is_some();
        return match Key::ec_from_parts(curve, d.as_deref(), x.as_deref(), y.as_deref()) {
            Ok(k) => Ok(new_key(ctx, Rc::new(k), private)),
            Err(_) => Ok(Value::FALSE),
        };
    }
    // DSA, DH and the 448-bit curves are not implemented: no key, as when
    // OpenSSL cannot make one from the parts given.
    for name in ["dsa", "dh", "x448", "ed448"] {
        if option(args, 0, name).is_some_and(|v| v.is_array()) {
            push_error(ctx, UNSUPPORTED);
            return Ok(Value::FALSE);
        }
    }
    for (name, x) in [("ed25519", false), ("x25519", true)] {
        if let Some(Ok(data)) = option(args, 0, name).map(Value::into_array) {
            let private = bn(&data, "priv_key");
            let public = bn(&data, "pub_key");
            let key = match (private, public) {
                (Some(p), _) => {
                    let Ok(p): Result<[u8; 32], _> = p.try_into() else { return Ok(Value::FALSE) };
                    if x { Key::x25519_from_secret(p) } else { Key::ed25519_from_seed(p) }
                }
                (None, Some(p)) => {
                    let Ok(p): Result<[u8; 32], _> = p.try_into() else { return Ok(Value::FALSE) };
                    let k = if x { Key::X25519 { public: p, secret: None } } else { Key::Ed25519 { public: p, seed: None } };
                    return Ok(new_key(ctx, Rc::new(k), false));
                }
                (None, None) => {
                    if x {
                        Key::generate_x25519()
                    } else {
                        Key::generate_ed25519()
                    }
                }
            };
            return Ok(new_key(ctx, Rc::new(key), true));
        }
    }
    let kind = option(args, 0, "private_key_type").map_or(pkey::KEYTYPE_RSA, |v| v.to_int());
    let bits = option(args, 0, "private_key_bits").map_or(2048, |v| v.to_int());
    if matches!(kind, pkey::KEYTYPE_RSA | pkey::KEYTYPE_DSA | pkey::KEYTYPE_DH) && bits < 384 {
        warn(ctx, &format!("Private key length must be at least 384 bits, configured to {bits}"));
        return Ok(Value::FALSE);
    }
    let key = match kind {
        pkey::KEYTYPE_RSA => match Key::generate_rsa(bits as usize, 65537) {
            Ok(k) => k,
            Err(e) => {
                push_error(ctx, e.queued());
                return Ok(Value::FALSE);
            }
        },
        pkey::KEYTYPE_EC => {
            let Some(name) = option(args, 0, "curve_name").and_then(|v| v.into_str().ok()) else {
                warn(ctx, "Missing configuration value: \"curve_name\" not set");
                return Ok(Value::FALSE);
            };
            let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
            match pkey::Curve::by_name(&name) {
                Some(c) => Key::generate_ec(c),
                None => {
                    warn(ctx, &format!("Unknown elliptic curve (short) name {name}"));
                    return Ok(Value::FALSE);
                }
            }
        }
        pkey::KEYTYPE_ED25519 => Key::generate_ed25519(),
        pkey::KEYTYPE_X25519 => Key::generate_x25519(),
        _ => {
            warn(ctx, "Unsupported private key type");
            return Ok(Value::FALSE);
        }
    };
    Ok(new_key(ctx, Rc::new(key), true))
}

/// The PEM openssl_pkey_export() writes (None: it failed, warned).
fn export_pem(ctx: &mut dyn Ctx, args: &[Value], out_arg: usize) -> Result<Option<String>, Throw> {
    let pass = opt_bytes(args, 2);
    let Some(key) = key_from_value(ctx, &arg(args, 0), false, pass.clone())? else {
        warn(ctx, "Cannot get key from parameter 1");
        return Ok(None);
    };
    let _ = out_arg;
    let encrypt = option(args, 3, "encrypt_key").is_none_or(|v| v.to_bool());
    let cipher = match option(args, 3, "encrypt_key_cipher").and_then(|v| v.as_int()) {
        Some(c) => match c {
            5 => "aes-128-cbc",
            6 => "aes-192-cbc",
            7 => "aes-256-cbc",
            4 => "des-ede3-cbc",
            _ => {
                warn(ctx, "Unknown cipher algorithm for private key");
                return Ok(None);
            }
        },
        _ => "des-ede3-cbc",
    };
    let enc = match &pass {
        Some(p) if encrypt => Some((cipher, &p[..])),
        _ => None,
    };
    Ok(key.private_pem(enc))
}

fn openssl_pkey_export(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    match export_pem(ctx, args, 1)? {
        Some(pem) => {
            set_ref(args, 1, s(pem));
            Ok(Value::TRUE)
        }
        None => Ok(Value::FALSE),
    }
}

fn openssl_pkey_export_to_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(pem) = export_pem(ctx, args, 1)? else { return Ok(Value::FALSE) };
    let Some(path) = output_path(ctx, &bytes(args, 1), "output_filename") else { return Ok(Value::FALSE) };
    if std::fs::write(&path, pem).is_err() {
        push_error(ctx, "error:80000002:system library::No such file or directory");
        return Ok(Value::FALSE);
    }
    Ok(Value::TRUE)
}

fn openssl_pkey_derive(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let len = arg(args, 2).to_int();
    if len < 0 {
        return Err(value_error(ctx, "Argument #3 ($key_length) must be greater than or equal to 0".into()));
    }
    let Some(own) = key_from_value(ctx, &arg(args, 1), false, Some(Vec::new()))? else { return Ok(Value::FALSE) };
    let Some(peer) = key_from_value(ctx, &arg(args, 0), true, None)? else { return Ok(Value::FALSE) };
    match own.derive(&peer) {
        Ok(mut secret) => {
            if len > 0 && (len as usize) < secret.len() {
                secret.truncate(len as usize);
            }
            Ok(s(secret))
        }
        Err(e) => {
            push_error(ctx, e.queued());
            Ok(Value::FALSE)
        }
    }
}

/// The digest openssl_sign/openssl_verify use: Ok(None) for the key's default.
fn sign_md(args: &[Value], i: usize) -> Result<Option<digest::Md>, ()> {
    if !given(args, i) {
        return Ok(Some(digest::Md::Sha1));
    }
    let v = arg(args, i);
    if let Some(name) = v.as_str() {
        return digest::by_name(name.as_bytes()).map(Some).ok_or(());
    }
    let n = v.to_int();
    if n == 0 {
        return Ok(None);
    }
    digest::by_algo(n).map(Some).ok_or(())
}

fn openssl_sign(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(key) = key_from_value(ctx, &arg(args, 2), false, Some(Vec::new()))? else {
        warn(ctx, "Supplied key param cannot be coerced into a private key");
        return Ok(Value::FALSE);
    };
    let Ok(md) = sign_md(args, 3) else {
        warn(ctx, "Unknown digest algorithm");
        return Ok(Value::FALSE);
    };
    if md.is_some_and(|m| !m.usable()) {
        push_error(ctx, UNSUPPORTED);
        return Ok(Value::FALSE);
    }
    match key.sign(md, &bytes(args, 0)) {
        Ok(sig) => {
            set_ref(args, 1, s(sig));
            Ok(Value::TRUE)
        }
        Err(e) => {
            push_error(ctx, e.queued());
            Ok(Value::FALSE)
        }
    }
}

fn openssl_verify(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(md) = sign_md(args, 3) else {
        warn(ctx, "Unknown digest algorithm");
        return Ok(Value::FALSE);
    };
    let Some(key) = key_from_value(ctx, &arg(args, 2), true, None)? else {
        warn(ctx, "Supplied key param cannot be coerced into a public key");
        return Ok(Value::FALSE);
    };
    Ok(Value::int(key.verify(md, &bytes(args, 0), &bytes(args, 1))))
}

/// An RSA operation with a padding (Key::public_encrypt and the rest).
type RsaOp = fn(&Key, &[u8], i64) -> Result<Vec<u8>, pkey::KeyError>;

fn rsa_op(ctx: &mut dyn Ctx, args: &mut [Value], public: bool, missing: &str, op: RsaOp) -> R {
    let pass = if public { None } else { Some(Vec::new()) };
    let Some(key) = key_from_value(ctx, &arg(args, 2), public, pass)? else {
        warn(ctx, missing);
        return Ok(Value::FALSE);
    };
    let padding = if given(args, 3) { arg(args, 3).to_int() } else { pkey::PKCS1_PADDING };
    match op(&key, &bytes(args, 0), padding) {
        Ok(out) => {
            set_ref(args, 1, s(out));
            Ok(Value::TRUE)
        }
        Err(e) => {
            push_error(ctx, e.queued());
            Ok(Value::FALSE)
        }
    }
}

fn openssl_public_encrypt(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    rsa_op(ctx, args, true, "key parameter is not a valid public key", Key::public_encrypt)
}

fn openssl_public_decrypt(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    rsa_op(ctx, args, true, "key parameter is not a valid public key", Key::public_decrypt)
}

fn openssl_private_encrypt(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    rsa_op(ctx, args, false, "key param is not a valid private key", Key::private_encrypt)
}

fn openssl_private_decrypt(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    rsa_op(ctx, args, false, "key parameter is not a valid private key", Key::private_decrypt)
}

/// openssl_seal: the data under a random key, that key under each public key.
fn openssl_seal(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(keys) = arg(args, 3).into_array() else { return Ok(Value::FALSE) };
    if keys.is_empty() {
        return Err(value_error(ctx, "Argument #4 ($public_key) must not be empty".into()));
    }
    let method = bytes(args, 4);
    let Some(spec) = cipher::by_name(&method) else {
        warn(ctx, "Unknown cipher algorithm");
        return Ok(Value::FALSE);
    };
    if args.len() < 6 && spec.iv_len > 0 {
        return Err(value_error(ctx, "Argument #6 ($iv) cannot be null for the chosen cipher algorithm".into()));
    }
    let mut pubkeys = Vec::new();
    for (i, v) in keys.values().enumerate() {
        match key_from_value(ctx, &v.deref(), true, None)? {
            Some(k) => pubkeys.push(k),
            None => {
                warn(ctx, &format!("Not a public key ({}th member of pubkeys)", i + 1));
                return Ok(Value::FALSE);
            }
        }
    }
    let secret = pkey::random(spec.key_len);
    let iv = pkey::random(spec.iv_len);
    let job = cipher::Job { spec, key: &secret, iv: &iv, data: &bytes(args, 0), aad: &[], pad: true, tag_len: 16, tag: &[] };
    let Ok((sealed, _)) = cipher::encrypt(&job) else { return Ok(Value::FALSE) };
    let mut ekeys = Vec::new();
    for k in &pubkeys {
        match k.public_encrypt(&secret, pkey::PKCS1_PADDING) {
            Ok(e) => ekeys.push(s(e)),
            Err(e) => {
                push_error(ctx, e.queued());
                return Ok(Value::FALSE);
            }
        }
    }
    if !sealed.is_empty() {
        set_ref(args, 1, s(sealed.clone()));
        set_ref(args, 2, Value::array(PhpArray::from_list(ekeys)));
        set_ref(args, 5, s(iv));
    }
    Ok(Value::int(sealed.len() as i64))
}

fn openssl_open(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(key) = key_from_value(ctx, &arg(args, 3), false, Some(Vec::new()))? else {
        warn(ctx, "Unable to coerce parameter 4 into a private key");
        return Ok(Value::FALSE);
    };
    let method = bytes(args, 4);
    let Some(spec) = cipher::by_name(&method) else {
        warn(ctx, "Unknown cipher algorithm");
        return Ok(Value::FALSE);
    };
    let iv = opt_bytes(args, 5);
    if spec.iv_len > 0 {
        let Some(iv) = &iv else {
            return Err(value_error(ctx, "Argument #6 ($iv) cannot be null for the chosen cipher algorithm".into()));
        };
        if iv.len() != spec.iv_len {
            warn(ctx, "IV length is invalid");
            return Ok(Value::FALSE);
        }
    }
    let iv = iv.unwrap_or_default();
    let secret = match key.private_decrypt(&bytes(args, 2), pkey::PKCS1_PADDING) {
        Ok(s) if s.len() == spec.key_len => s,
        Ok(_) => return Ok(Value::FALSE),
        Err(e) => {
            push_error(ctx, e.queued());
            return Ok(Value::FALSE);
        }
    };
    let job = cipher::Job { spec, key: &secret, iv: &iv, data: &bytes(args, 0), aad: &[], pad: true, tag_len: 0, tag: &[] };
    match cipher::decrypt(&job) {
        Ok(out) if !out.is_empty() => {
            set_ref(args, 1, s(out));
            Ok(Value::TRUE)
        }
        Ok(_) => Ok(Value::FALSE),
        Err(e) => {
            if let Some(q) = e.queued() {
                push_error(ctx, q);
            }
            Ok(Value::FALSE)
        }
    }
}

// ---- symmetric ciphers ----

const RAW_DATA: i64 = 1;
const ZERO_PADDING: i64 = 2;
const DONT_ZERO_PAD_KEY: i64 = 4;

/// PHP's non-strict base64_decode: other characters skipped, bits left
/// over at the end dropped; it never fails.
fn php_base64_decode(data: &[u8]) -> Vec<u8> {
    let clean: Vec<u8> = data.iter().copied().filter(|c| c.is_ascii_alphanumeric() || *c == b'+' || *c == b'/').collect();
    der::base64_decode(&clean).unwrap_or_default()
}

/// What an AEAD tag is to openssl_encrypt (one of this length to make) and
/// openssl_decrypt (the one given, to check).
enum Tag<'a> {
    Make(usize),
    Check(Option<&'a [u8]>),
}

/// The steps of php_openssl_cipher_init that can fail with a warning:
/// the key and IV fitted to the cipher.
fn prepare(ctx: &mut dyn Ctx, spec: &cipher::Spec, key: &[u8], iv: &[u8], options: i64, tag: Tag) -> Option<(Vec<u8>, Vec<u8>)> {
    let (enc, tag, tag_len) = match tag {
        Tag::Make(len) => (true, None, len),
        Tag::Check(tag) => (false, tag, 0),
    };
    if enc && iv.is_empty() && spec.iv_len > 0 && !spec.is_aead() {
        warn(ctx, "Using an empty Initialization Vector (iv) is potentially insecure and not recommended");
    }
    let iv = if spec.is_aead() {
        if !spec.iv_len_ok(iv.len()) {
            push_error(ctx, "error:1C80006D:Provider routines::invalid iv length");
            warn(ctx, "Setting of IV length for AEAD mode failed");
            return None;
        }
        iv.to_vec()
    } else if iv.len() == spec.iv_len {
        iv.to_vec()
    } else if iv.is_empty() {
        vec![0; spec.iv_len]
    } else if iv.len() < spec.iv_len {
        warn(ctx, &format!("IV passed is only {} bytes long, cipher expects an IV of precisely {} bytes, padding with \\0", iv.len(), spec.iv_len));
        let mut v = iv.to_vec();
        v.resize(spec.iv_len, 0);
        v
    } else {
        warn(ctx, &format!("IV passed is {} bytes long which is longer than the {} expected by selected cipher, truncating", iv.len(), spec.iv_len));
        iv[..spec.iv_len].to_vec()
    };
    let ocb_tag_len = if enc { tag_len } else { tag.map_or(0, <[u8]>::len) };
    if spec.mode == cipher::Mode::Ocb && !(1..=16).contains(&ocb_tag_len) {
        warn(ctx, "Setting tag length for AEAD cipher failed");
        return None;
    }
    if enc && spec.mode == cipher::Mode::Ccm && !cipher::ccm_tag_len_ok(tag_len) {
        warn(ctx, "Setting tag length for AEAD cipher failed");
        return None;
    }
    if let Some(tag) = tag.filter(|t| !t.is_empty())
        && !enc
    {
        if !spec.is_aead() {
            warn(ctx, "The tag cannot be used because the cipher algorithm does not support AEAD");
        } else if tag.len() > 16 || (spec.mode == cipher::Mode::Ccm && !cipher::ccm_tag_len_ok(tag.len())) {
            warn(ctx, "Setting tag for AEAD cipher decryption failed");
            return None;
        }
    }
    let key = if key.len() < spec.key_len {
        if options & DONT_ZERO_PAD_KEY != 0 {
            push_error(ctx, "error:03000082:digital envelope routines::invalid key length");
            warn(ctx, "Key length cannot be set for the cipher algorithm");
            return None;
        }
        let mut k = key.to_vec();
        k.resize(spec.key_len, 0);
        k
    } else {
        if key.len() > spec.key_len {
            push_error(ctx, "error:03000082:digital envelope routines::invalid key length");
        }
        key[..spec.key_len].to_vec()
    };
    Some((key, iv))
}

fn openssl_encrypt(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let method = bytes(args, 1);
    let Some(spec) = cipher::by_name(&method) else {
        warn(ctx, "Unknown cipher algorithm");
        return Ok(Value::FALSE);
    };
    let options = arg(args, 3).to_int();
    let iv = bytes(args, 4);
    let tag_len = if given(args, 7) { arg(args, 7).to_int() } else { 16 };
    let has_tag = args.len() > 5;
    let Some((key, iv)) = prepare(ctx, &spec, &bytes(args, 2), &iv, options, Tag::Make(tag_len.max(0) as usize)) else {
        return Ok(Value::FALSE);
    };
    let data = bytes(args, 0);
    let aad = bytes(args, 6);
    let job = cipher::Job { spec, key: &key, iv: &iv, data: &data, aad: &aad, pad: options & ZERO_PADDING == 0, tag_len: tag_len.max(0) as usize, tag: &[] };
    let (out, full_tag) = match cipher::encrypt(&job) {
        Ok(r) => r,
        Err(e) => {
            if let Some(q) = e.queued() {
                push_error(ctx, q);
            }
            return Ok(Value::FALSE);
        }
    };
    let out = if options & RAW_DATA != 0 { out } else { der::base64_encode(&out).into_bytes() };
    if spec.is_aead() && has_tag {
        let ok = match spec.mode {
            cipher::Mode::Ccm | cipher::Mode::Ocb => tag_len as usize == full_tag.len(),
            _ => (1..=16).contains(&tag_len),
        };
        if !ok {
            push_error(ctx, "error:1C80006E:Provider routines::invalid tag");
            warn(ctx, "Retrieving verification tag failed");
            return Ok(Value::FALSE);
        }
        set_ref(args, 5, s(full_tag[..tag_len as usize].to_vec()));
    } else if has_tag {
        set_ref(args, 5, Value::NULL);
    } else if spec.is_aead() {
        warn(ctx, "A tag should be provided when using AEAD mode");
        return Ok(Value::FALSE);
    }
    Ok(s(out))
}

fn openssl_decrypt(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let method = bytes(args, 1);
    if method.is_empty() {
        return Err(value_error(ctx, "Argument #2 ($cipher_algo) must not be empty".into()));
    }
    let Some(spec) = cipher::by_name(&method) else {
        warn(ctx, "Unknown cipher algorithm");
        return Ok(Value::FALSE);
    };
    let options = arg(args, 3).to_int();
    let mut data = bytes(args, 0);
    if options & RAW_DATA == 0 {
        data = php_base64_decode(&data);
    }
    let tag = opt_bytes(args, 5);
    let Some((key, iv)) = prepare(ctx, &spec, &bytes(args, 2), &bytes(args, 4), options, Tag::Check(tag.as_deref())) else {
        return Ok(Value::FALSE);
    };
    let aad = bytes(args, 6);
    let tag = tag.unwrap_or_default();
    let job = cipher::Job { spec, key: &key, iv: &iv, data: &data, aad: &aad, pad: options & ZERO_PADDING == 0, tag_len: tag.len(), tag: &tag };
    match cipher::decrypt(&job) {
        Ok(out) => Ok(s(out)),
        Err(e) => {
            if let Some(q) = e.queued() {
                push_error(ctx, q);
            }
            Ok(Value::FALSE)
        }
    }
}

fn cipher_length(ctx: &mut dyn Ctx, args: &[Value], iv: bool) -> R {
    let method = bytes(args, 0);
    if method.is_empty() {
        return Err(value_error(ctx, "Argument #1 ($cipher_algo) must not be empty".into()));
    }
    match cipher::by_name(&method) {
        Some(spec) => Ok(Value::int(if iv { spec.iv_len } else { spec.key_len } as i64)),
        None => {
            warn(ctx, "Unknown cipher algorithm");
            Ok(Value::FALSE)
        }
    }
}

fn openssl_cipher_iv_length(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    cipher_length(ctx, args, true)
}

fn openssl_cipher_key_length(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    cipher_length(ctx, args, false)
}

// ---- digests, randomness, lists ----

fn openssl_digest(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(md) = digest::by_name(&bytes(args, 1)) else {
        warn(ctx, "Unknown digest algorithm");
        return Ok(Value::FALSE);
    };
    if !md.usable() {
        push_error(ctx, UNSUPPORTED);
        return Ok(Value::FALSE);
    }
    let h = md.hash(&bytes(args, 0));
    Ok(s(if arg(args, 2).to_bool() { h } else { hex(&h) }))
}

fn openssl_pbkdf2(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let len = arg(args, 2).to_int();
    let iterations = arg(args, 3).to_int();
    if len <= 0 {
        return Err(value_error(ctx, "Argument #3 ($key_length) must be greater than 0".into()));
    }
    if iterations <= 0 {
        return Err(value_error(ctx, "Argument #4 ($iterations) must be greater than 0".into()));
    }
    let name = if given(args, 4) { bytes(args, 4) } else { b"sha1".to_vec() };
    let Some(md) = digest::by_name(&name).filter(|m| m.has_hmac() && m.usable()) else {
        warn(ctx, "Unknown digest algorithm");
        return Ok(Value::FALSE);
    };
    Ok(s(md.pbkdf2(&bytes(args, 0), &bytes(args, 1), iterations.min(i64::from(u32::MAX)) as u32, len as usize)))
}

fn list(names: Vec<&'static str>) -> Value {
    Value::array(PhpArray::from_list(names.into_iter().map(s).collect()))
}

fn openssl_get_md_methods(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(list(digest::method_names(arg(args, 0).to_bool())))
}

fn openssl_get_cipher_methods(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(list(cipher::method_names(arg(args, 0).to_bool())))
}

fn openssl_get_curve_names(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(list(pkey::CURVES.iter().map(|c| c.1).collect()))
}

fn openssl_random_pseudo_bytes(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let len = arg(args, 0).to_int();
    if len <= 0 {
        set_ref(args, 1, Value::FALSE);
        return Err(value_error(ctx, "Argument #1 ($length) must be greater than 0".into()));
    }
    if len > i64::from(i32::MAX) {
        set_ref(args, 1, Value::FALSE);
        return Err(value_error(ctx, format!("Argument #1 ($length) must be less than or equal to {}", i32::MAX)));
    }
    let out = pkey::random(len as usize);
    set_ref(args, 1, Value::TRUE);
    Ok(s(out))
}
