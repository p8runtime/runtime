//! Hashes: md5, sha1, crc32, hash() and hash_hmac() with every algorithm
//! PHP 8.4 has (the RustCrypto crates and xxhash where they agree with
//! PHP, hash_algos.rs for the rest), hash_equals, hash_algos, and the
//! _file forms.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/md5.c, sha1.c, crc32.c,
//! $PHP_SRC_PATH/ext/hash/hash.c

use crate::{arg, str_arg};
use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Throw};
use p8_runtime::object::{CLASS_FINAL, Object, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "md5", run: md5, by_ref: &[] },
    Builtin { name: "sha1", run: sha1, by_ref: &[] },
    Builtin { name: "crc32", run: crc32, by_ref: &[] },
    Builtin { name: "md5_file", run: md5_file, by_ref: &[] },
    Builtin { name: "sha1_file", run: sha1_file, by_ref: &[] },
    Builtin { name: "hash", run: hash, by_ref: &[] },
    Builtin { name: "hash_file", run: hash_file, by_ref: &[] },
    Builtin { name: "hash_hmac", run: hash_hmac, by_ref: &[] },
    Builtin { name: "hash_algos", run: hash_algos, by_ref: &[] },
    Builtin { name: "hash_hmac_algos", run: hash_hmac_algos, by_ref: &[] },
    Builtin { name: "hash_equals", run: hash_equals, by_ref: &[] },
    Builtin { name: "hash_init", run: hash_init, by_ref: &[] },
    Builtin { name: "hash_update", run: hash_update, by_ref: &[] },
    Builtin { name: "hash_update_stream", run: hash_update_stream, by_ref: &[] },
    Builtin { name: "hash_update_file", run: hash_update_file, by_ref: &[] },
    Builtin { name: "hash_final", run: hash_final, by_ref: &[] },
    Builtin { name: "hash_copy", run: hash_copy, by_ref: &[] },
    Builtin { name: "hash_hkdf", run: hash_hkdf, by_ref: &[] },
    Builtin { name: "hash_pbkdf2", run: hash_pbkdf2, by_ref: &[] },
];

/// hash_algos(), in PHP's order.
#[rustfmt::skip]
const ALGOS: &[&str] = &[
    "md2", "md4", "md5", "sha1", "sha224", "sha256", "sha384", "sha512/224", "sha512/256", "sha512",
    "sha3-224", "sha3-256", "sha3-384", "sha3-512", "ripemd128", "ripemd160", "ripemd256", "ripemd320",
    "whirlpool", "tiger128,3", "tiger160,3", "tiger192,3", "tiger128,4", "tiger160,4", "tiger192,4",
    "snefru", "snefru256", "gost", "gost-crypto", "adler32", "crc32", "crc32b", "crc32c", "fnv132",
    "fnv1a32", "fnv164", "fnv1a64", "joaat", "murmur3a", "murmur3c", "murmur3f", "xxh32", "xxh64",
    "xxh3", "xxh128", "haval128,3", "haval160,3", "haval192,3", "haval224,3", "haval256,3",
    "haval128,4", "haval160,4", "haval192,4", "haval224,4", "haval256,4", "haval128,5", "haval160,5",
    "haval192,5", "haval224,5", "haval256,5",
];

/// Algorithm `name`'s digest of `data`, or None for one not known.
fn digest(name: &str, data: &[u8]) -> Option<Vec<u8>> {
    digest_seeded(name, data, None)
}

/// "tiger192,4" / "haval256,5": the bits and passes.
fn bits_passes(name: &str, prefix: &str) -> Option<(u32, usize)> {
    let (bits, passes) = name.strip_prefix(prefix)?.split_once(',')?;
    Some((bits.parse().ok()?, passes.parse().ok()?))
}

/// As digest, with the `seed` option murmur and xxhash take.
fn digest_seeded(name: &str, data: &[u8], seed: Option<u64>) -> Option<Vec<u8>> {
    use crate::hash_algos as alg;
    use sha2::Digest;
    if let Some((bits, passes)) = bits_passes(name, "tiger") {
        return Some(alg::tiger(data, bits as usize, passes));
    }
    if let Some((bits, passes)) = bits_passes(name, "haval") {
        return Some(alg::haval(data, bits, passes));
    }
    let seed32 = seed.unwrap_or(0) as u32;
    Some(match name {
        "md2" => md2::Md2::digest(data).to_vec(),
        "md4" => md4::Md4::digest(data).to_vec(),
        "sha3-224" => sha3::Sha3_224::digest(data).to_vec(),
        "sha3-256" => sha3::Sha3_256::digest(data).to_vec(),
        "sha3-384" => sha3::Sha3_384::digest(data).to_vec(),
        "sha3-512" => sha3::Sha3_512::digest(data).to_vec(),
        "ripemd128" => ripemd::Ripemd128::digest(data).to_vec(),
        "ripemd160" => ripemd::Ripemd160::digest(data).to_vec(),
        "ripemd256" => ripemd::Ripemd256::digest(data).to_vec(),
        "ripemd320" => ripemd::Ripemd320::digest(data).to_vec(),
        "whirlpool" => whirlpool::Whirlpool::digest(data).to_vec(),
        "snefru" | "snefru256" => alg::snefru(data),
        "gost" => gost94::Gost94Test::digest(data).to_vec(),
        "gost-crypto" => gost94::Gost94CryptoPro::digest(data).to_vec(),
        "crc32" => alg::crc32_bzip2(data),
        "crc32c" => alg::crc32c(data),
        "murmur3a" => alg::murmur3a(data, seed32),
        "murmur3c" => alg::murmur3c(data, seed32),
        "murmur3f" => alg::murmur3f(data, seed.unwrap_or(0)),
        "xxh32" => xxhash_rust::xxh32::xxh32(data, seed32).to_be_bytes().to_vec(),
        "xxh64" => xxhash_rust::xxh64::xxh64(data, seed.unwrap_or(0)).to_be_bytes().to_vec(),
        "xxh3" => xxhash_rust::xxh3::xxh3_64_with_seed(data, seed.unwrap_or(0)).to_be_bytes().to_vec(),
        "xxh128" => xxhash_rust::xxh3::xxh3_128_with_seed(data, seed.unwrap_or(0)).to_be_bytes().to_vec(),
        "md5" => md5::Md5::digest(data).to_vec(),
        "sha1" => sha1::Sha1::digest(data).to_vec(),
        "sha224" => sha2::Sha224::digest(data).to_vec(),
        "sha256" => sha2::Sha256::digest(data).to_vec(),
        "sha384" => sha2::Sha384::digest(data).to_vec(),
        "sha512/224" => sha2::Sha512_224::digest(data).to_vec(),
        "sha512/256" => sha2::Sha512_256::digest(data).to_vec(),
        "sha512" => sha2::Sha512::digest(data).to_vec(),
        "crc32b" => crc32fast::hash(data).to_be_bytes().to_vec(),
        "adler32" => {
            let (mut a, mut b) = (1u32, 0u32);
            for &c in data {
                a = (a + u32::from(c)) % 65521;
                b = (b + a) % 65521;
            }
            ((b << 16) | a).to_be_bytes().to_vec()
        }
        "fnv132" | "fnv1a32" => {
            let mut h: u32 = 0x811c_9dc5;
            for &c in data {
                if name == "fnv132" {
                    h = h.wrapping_mul(0x0100_0193) ^ u32::from(c);
                } else {
                    h = (h ^ u32::from(c)).wrapping_mul(0x0100_0193);
                }
            }
            h.to_be_bytes().to_vec()
        }
        "fnv164" | "fnv1a64" => {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for &c in data {
                if name == "fnv164" {
                    h = h.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(c);
                } else {
                    h = (h ^ u64::from(c)).wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
            h.to_be_bytes().to_vec()
        }
        "joaat" => {
            let mut h: u32 = 0;
            for &c in data {
                h = h.wrapping_add(u32::from(c));
                h = h.wrapping_add(h << 10);
                h ^= h >> 6;
            }
            h = h.wrapping_add(h << 3);
            h ^= h >> 11;
            h = h.wrapping_add(h << 15);
            h.to_be_bytes().to_vec()
        }
        _ => return None,
    })
}

/// Block size for HMAC.
fn block_size(name: &str) -> usize {
    match name {
        "sha384" | "sha512" | "sha512/224" | "sha512/256" => 128,
        "md2" => 16,
        "sha3-224" => 144,
        "sha3-256" => 136,
        "sha3-384" => 104,
        "sha3-512" => 72,
        "snefru" | "snefru256" | "gost" | "gost-crypto" => 32,
        n if n.starts_with("haval") => 128,
        _ => 64,
    }
}

fn hex(bytes: &[u8]) -> Vec<u8> {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes.iter().flat_map(|b| [DIGITS[(b >> 4) as usize], DIGITS[(b & 15) as usize]]).collect()
}

/// The digest as a binary string, or its hex (`binary` false).
fn shown(d: Vec<u8>, binary: bool) -> Value {
    Value::str(PhpStr::new(if binary { d } else { hex(&d) }))
}

fn binary(args: &[Value], i: usize) -> bool {
    args.len() > i && arg(args, i).to_bool()
}

fn md5(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(shown(digest("md5", str_arg(args, 0).as_bytes()).expect("md5"), binary(args, 1)))
}

fn sha1(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(shown(digest("sha1", str_arg(args, 0).as_bytes()).expect("sha1"), binary(args, 1)))
}

/// crc32(): the checksum as an int.
fn crc32(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::int(i64::from(crc32fast::hash(str_arg(args, 0).as_bytes()))))
}

fn read_file(ctx: &mut dyn Ctx, func: &str, path: &PhpStr) -> Option<Vec<u8>> {
    match std::fs::read(String::from_utf8_lossy(path.as_bytes()).as_ref()) {
        Ok(b) => Some(b),
        Err(_) => {
            ctx.warn(&format!("{func}({}): Failed to open stream: No such file or directory", String::from_utf8_lossy(path.as_bytes())));
            None
        }
    }
}

fn md5_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(b) = read_file(ctx, "md5_file", &str_arg(args, 0)) else { return Ok(Value::FALSE) };
    Ok(shown(digest("md5", &b).expect("md5"), binary(args, 1)))
}

fn sha1_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(b) = read_file(ctx, "sha1_file", &str_arg(args, 0)) else { return Ok(Value::FALSE) };
    Ok(shown(digest("sha1", &b).expect("sha1"), binary(args, 1)))
}

fn algo(ctx: &mut dyn Ctx, func: &str, args: &[Value]) -> Result<String, Throw> {
    let name = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).to_ascii_lowercase();
    if !ALGOS.contains(&name.as_str()) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($algo) must be a valid hashing algorithm")));
    }
    Ok(name)
}

/// The `seed` in an options array (murmur, xxhash).
fn seed_option(args: &[Value], i: usize) -> Option<u64> {
    args.get(i).map(Value::deref)?.as_array().and_then(|a| a.get(&p8_runtime::array::Key::from_string(PhpStr::from("seed"))).map(|v| v.to_int() as u64))
}

/// hash(string $algo, string $data, bool $binary = false, array $options = [])
fn hash(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = algo(ctx, "hash", args)?;
    Ok(shown(digest_seeded(&name, str_arg(args, 1).as_bytes(), seed_option(args, 3)).expect("a known algorithm"), binary(args, 2)))
}

fn hash_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = algo(ctx, "hash_file", args)?;
    let Some(b) = read_file(ctx, "hash_file", &str_arg(args, 1)) else { return Ok(Value::FALSE) };
    Ok(shown(digest(&name, &b).expect("a known algorithm"), binary(args, 2)))
}

/// Whether algorithm `name` is a checksum (no HMAC, HKDF or PBKDF2 with it).
fn not_cryptographic(name: &str) -> bool {
    matches!(
        name,
        "adler32" | "crc32" | "crc32b" | "crc32c" | "fnv132" | "fnv1a32" | "fnv164" | "fnv1a64" | "joaat" | "murmur3a" | "murmur3c" | "murmur3f" | "xxh32" | "xxh64" | "xxh3" | "xxh128"
    )
}

/// HMAC of `data` with `key` (RFC 2104).
fn hmac(name: &str, key: &[u8], data: &[u8]) -> Vec<u8> {
    let block = block_size(name);
    let mut k = key.to_vec();
    if k.len() > block {
        k = digest(name, &k).expect("a known algorithm");
    }
    k.resize(block, 0);
    let inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).chain(data.iter().copied()).collect();
    let inner = digest(name, &inner).expect("a known algorithm");
    let outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).chain(inner).collect();
    digest(name, &outer).expect("a known algorithm")
}

/// hash_hmac(string $algo, string $data, string $key, bool $binary = false)
fn hash_hmac(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = algo(ctx, "hash_hmac", args)?;
    if not_cryptographic(&name) {
        return Err(ctx.throw("ValueError", "hash_hmac(): Argument #1 ($algo) must be a valid cryptographic hashing algorithm".into()));
    }
    Ok(shown(hmac(&name, str_arg(args, 2).as_bytes(), str_arg(args, 1).as_bytes()), binary(args, 3)))
}

// ---------------------------------------------------------------------------
// HashContext: hash_init, hash_update…, hash_final, hash_copy. A context
// keeps what it was given and digests it at hash_final.

const ALGO: &str = "\0algo";
const DATA: &str = "\0data";
const KEY: &str = "\0key";
const DONE: &str = "\0done";

pub static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "HashContext",
    parent: None,
    interfaces: &[],
    flags: CLASS_FINAL,
    properties: &[
        (ALGO, Constant::Null, Visibility::Public),
        (DATA, Constant::Str(b""), Visibility::Public),
        (KEY, Constant::Null, Visibility::Public),
        (DONE, Constant::Bool(false), Visibility::Public),
    ],
    methods: &[],
    constants: &[],
    debug_info: None,
}];

/// The HashContext in argument 0, not yet finalized.
fn context(ctx: &mut dyn Ctx, func: &str, args: &[Value]) -> Result<Rc<Object>, Throw> {
    let v = arg(args, 0);
    match v.as_object() {
        Some(o) if o.class.name.as_bytes() == b"HashContext" => {
            if crate::spl::get(o, DONE).to_bool() {
                return Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($context) must be a valid, non-finalized HashContext")));
            }
            Ok(Rc::clone(o))
        }
        _ => Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($context) must be of type HashContext, {} given", v.type_name()))),
    }
}

fn append(o: &Object, bytes: &[u8]) {
    let mut data = crate::spl::get(o, DATA).to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec();
    data.extend_from_slice(bytes);
    crate::spl::set(o, DATA, Value::str(PhpStr::new(data)));
}

/// hash_init(string $algo, int $flags = 0, string $key = "", array $options = [])
fn hash_init(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = algo(ctx, "hash_init", args)?;
    let hmac = args.len() > 1 && arg(args, 1).to_int() & 1 != 0;
    let key = str_arg(args, 2);
    if hmac {
        if not_cryptographic(&name) {
            return Err(ctx.throw("ValueError", "hash_init(): Argument #1 ($algo) must be a cryptographic hashing algorithm if HMAC is requested".into()));
        }
        if key.is_empty() {
            return Err(ctx.throw("ValueError", "hash_init(): Argument #3 ($key) must not be empty when HMAC is requested".into()));
        }
    }
    let class = ctx.class(b"HashContext").expect("HashContext is declared");
    let o = ctx.new_instance(&class, &mut [], false)?;
    crate::spl::set(&o, ALGO, Value::str(name.as_str()));
    if hmac {
        crate::spl::set(&o, KEY, Value::str(key));
    }
    Ok(Value::object(o))
}

/// hash_update(HashContext $context, string $data): true
fn hash_update(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context(ctx, "hash_update", args)?;
    append(&o, str_arg(args, 1).as_bytes());
    Ok(Value::TRUE)
}

/// hash_update_stream(HashContext $context, resource $stream, int $length = -1): bytes read
fn hash_update_stream(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context(ctx, "hash_update_stream", args)?;
    let s = crate::file::stream_arg(ctx, "hash_update_stream", args, 1)?;
    let length = if args.len() > 2 { arg(args, 2).to_int() } else { -1 };
    let r = p8_runtime::stream::resource(&s).expect("a stream resource");
    let bytes = if length < 0 { r.stream.borrow_mut().read_all() } else { r.stream.borrow_mut().read(length as usize) };
    append(&o, &bytes);
    Ok(Value::int(bytes.len() as i64))
}

/// hash_update_file(HashContext $context, string $filename, ?resource $stream_context = null)
fn hash_update_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context(ctx, "hash_update_file", args)?;
    let Some(bytes) = read_file(ctx, "hash_update_file", &str_arg(args, 1)) else { return Ok(Value::FALSE) };
    append(&o, &bytes);
    Ok(Value::TRUE)
}

/// hash_final(HashContext $context, bool $binary = false): the digest; the context is done.
fn hash_final(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context(ctx, "hash_final", args)?;
    let name = crate::spl::get(&o, ALGO).to_php_string().unwrap_or_else(PhpStr::empty);
    let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
    let data = crate::spl::get(&o, DATA).to_php_string().unwrap_or_else(PhpStr::empty);
    let d = match crate::spl::get(&o, KEY).as_str() {
        Some(key) => hmac(&name, key.as_bytes(), data.as_bytes()),
        None => digest(&name, data.as_bytes()).expect("a known algorithm"),
    };
    crate::spl::set(&o, DONE, Value::TRUE);
    crate::spl::set(&o, DATA, Value::str(""));
    Ok(shown(d, binary(args, 1)))
}

/// hash_copy(HashContext $context): another, with what this one has so far.
fn hash_copy(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context(ctx, "hash_copy", args)?;
    let copy = ctx.new_instance(&Rc::clone(&o.class), &mut [], false)?;
    for name in [ALGO, DATA, KEY, DONE] {
        crate::spl::set(&copy, name, crate::spl::get(&o, name));
    }
    Ok(Value::object(copy))
}

/// The algorithm in argument 0, for a function needing a cryptographic one.
fn crypto_algo(ctx: &mut dyn Ctx, func: &str, args: &[Value]) -> Result<String, Throw> {
    let name = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).to_ascii_lowercase();
    if !ALGOS.contains(&name.as_str()) || not_cryptographic(&name) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($algo) must be a valid cryptographic hashing algorithm")));
    }
    Ok(name)
}

/// hash_hkdf(string $algo, string $key, int $length = 0, string $info = "", string $salt = ""): RFC 5869.
fn hash_hkdf(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = crypto_algo(ctx, "hash_hkdf", args)?;
    let key = str_arg(args, 1);
    if key.is_empty() {
        return Err(ctx.throw("ValueError", "hash_hkdf(): Argument #2 ($key) must not be empty".into()));
    }
    let size = digest(&name, b"").expect("a known algorithm").len();
    let length = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    if length < 0 {
        return Err(ctx.throw("ValueError", "hash_hkdf(): Argument #3 ($length) must be greater than or equal to 0".into()));
    }
    if length > (255 * size) as i64 {
        return Err(ctx.throw("ValueError", format!("hash_hkdf(): Argument #3 ($length) must be less than or equal to {}", 255 * size)));
    }
    let length = if length == 0 { size } else { length as usize };
    let (info, salt) = (str_arg(args, 3), str_arg(args, 4));
    let salt = if salt.is_empty() { vec![0; size] } else { salt.as_bytes().to_vec() };
    let prk = hmac(&name, &salt, key.as_bytes());
    let mut out = Vec::with_capacity(length);
    let mut t: Vec<u8> = Vec::new();
    let mut i: u8 = 1;
    while out.len() < length {
        let input: Vec<u8> = t.iter().chain(info.as_bytes()).copied().chain([i]).collect();
        t = hmac(&name, &prk, &input);
        out.extend_from_slice(&t);
        i = i.wrapping_add(1);
    }
    out.truncate(length);
    Ok(Value::str(PhpStr::new(out)))
}

/// hash_pbkdf2(string $algo, string $password, string $salt, int $iterations, int $length = 0, bool $binary = false)
fn hash_pbkdf2(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = crypto_algo(ctx, "hash_pbkdf2", args)?;
    let (password, salt) = (str_arg(args, 1), str_arg(args, 2));
    let iterations = arg(args, 3).to_int();
    if iterations <= 0 {
        return Err(ctx.throw("ValueError", "hash_pbkdf2(): Argument #4 ($iterations) must be greater than 0".into()));
    }
    let length = if args.len() > 4 { arg(args, 4).to_int() } else { 0 };
    if length < 0 {
        return Err(ctx.throw("ValueError", "hash_pbkdf2(): Argument #5 ($length) must be greater than or equal to 0".into()));
    }
    let raw = binary(args, 5);
    let size = digest(&name, b"").expect("a known algorithm").len();
    // The length counts hex digits unless the output is binary.
    let want = match (length, raw) {
        (0, true) => size,
        (0, false) => size * 2,
        (n, _) => n as usize,
    };
    let bytes = if raw { want } else { want.div_ceil(2) };
    let mut out = Vec::with_capacity(bytes + size);
    let mut block: u32 = 1;
    while out.len() < bytes {
        let mut u = hmac(&name, password.as_bytes(), &[salt.as_bytes(), &block.to_be_bytes()].concat());
        let mut t = u.clone();
        for _ in 1..iterations {
            u = hmac(&name, password.as_bytes(), &u);
            for (x, y) in t.iter_mut().zip(&u) {
                *x ^= y;
            }
        }
        out.extend_from_slice(&t);
        block += 1;
    }
    if raw {
        out.truncate(want);
        Ok(Value::str(PhpStr::new(out)))
    } else {
        let mut h = hex(&out);
        h.truncate(want);
        Ok(Value::str(PhpStr::new(h)))
    }
}

fn hash_algos(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::array(PhpArray::from_list(ALGOS.iter().map(|a| Value::str(*a)).collect())))
}

/// hash_hmac_algos(): the cryptographic ones.
fn hash_hmac_algos(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::array(PhpArray::from_list(ALGOS.iter().filter(|a| !not_cryptographic(a)).map(|a| Value::str(*a)).collect())))
}

/// hash_equals(string $known_string, string $user_string): compared in constant time.
fn hash_equals(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    for (i, name) in [(0, "known_string"), (1, "user_string")] {
        let v = arg(args, i);
        if !v.is_str() {
            return Err(ctx.throw(
                "TypeError",
                format!("hash_equals(): Argument #{} (${name}) must be of type string, {} given", i + 1, v.type_name()),
            ));
        }
    }
    let (a, b) = (str_arg(args, 0), str_arg(args, 1));
    if a.len() != b.len() {
        return Ok(Value::FALSE);
    }
    let diff = a.as_bytes().iter().zip(b.as_bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    Ok(Value::bool(diff == 0))
}
