//! password_hash() and friends: bcrypt ("2y", the default) and argon2i /
//! argon2id, with PHP's salts, options, errors and encodings.
//!
//! bcrypt follows crypt_blowfish.c: the key schedule (BF_set_key, with the
//! $2x$ sign-extension bug and $2a$'s countermeasure) is computed here, and
//! RustCrypto's Blowfish does the eksblowfish rounds on those key words.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/password.c,
//! $PHP_SRC_PATH/ext/standard/crypt_blowfish.c

use crate::{arg, str_arg};
use argon2::{Algorithm, Argon2, Params, Version};
use blowfish::Blowfish;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "password_hash", run: password_hash, by_ref: &[] },
    Builtin { name: "password_verify", run: password_verify, by_ref: &[] },
    Builtin { name: "password_needs_rehash", run: password_needs_rehash, by_ref: &[] },
    Builtin { name: "password_get_info", run: password_get_info, by_ref: &[] },
    Builtin { name: "password_algos", run: password_algos, by_ref: &[] },
    Builtin { name: "crypt", run: crypt_fn, by_ref: &[] },
];

const BCRYPT_COST: i64 = 12;
const ARGON2_MEMORY_COST: i64 = 65536;
const ARGON2_TIME_COST: i64 = 4;
const ARGON2_THREADS: i64 = 1;

#[derive(Clone, Copy, PartialEq)]
enum Algo {
    Bcrypt,
    Argon2i,
    Argon2id,
}

impl Algo {
    fn ident(self) -> &'static str {
        match self {
            Algo::Bcrypt => "2y",
            Algo::Argon2i => "argon2i",
            Algo::Argon2id => "argon2id",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Algo::Bcrypt => "bcrypt",
            Algo::Argon2i => "argon2i",
            Algo::Argon2id => "argon2id",
        }
    }

    fn by_ident(ident: &[u8]) -> Option<Algo> {
        [Algo::Bcrypt, Algo::Argon2i, Algo::Argon2id].into_iter().find(|a| a.ident().as_bytes() == ident)
    }

    /// The algorithm a hash says it is (php_password_algo_identify_ex):
    /// its "$ident$" prefix, if known and (for bcrypt) well formed.
    fn of_hash(hash: &[u8]) -> Option<Algo> {
        let ident = extract_ident(hash)?;
        let algo = Algo::by_ident(ident)?;
        (algo != Algo::Bcrypt || bcrypt_valid(hash)).then_some(algo)
    }
}

/// php_password_algo_extract_ident(): what is between the first two `$`.
fn extract_ident(hash: &[u8]) -> Option<&[u8]> {
    if hash.len() < 3 {
        return None;
    }
    let rest = &hash[1..];
    let end = rest.iter().position(|&c| c == b'$')?;
    Some(&rest[..end])
}

fn bcrypt_valid(hash: &[u8]) -> bool {
    hash.len() == 60 && hash.starts_with(b"$2y")
}

/// The algorithm argument: null or 0 is the default, 1-3 the PASSWORD_*
/// numbers, a string an identifier. None: unknown.
fn algo_arg(v: &Value) -> Option<Algo> {
    match v {
        Value::Null => Some(Algo::Bcrypt),
        Value::Int(0 | 1) => Some(Algo::Bcrypt),
        Value::Int(2) => Some(Algo::Argon2i),
        Value::Int(3) => Some(Algo::Argon2id),
        Value::Int(_) => None,
        other => Algo::by_ident(other.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()),
    }
}

/// An option's value as an integer (zval_get_long), if the options have it.
fn option(options: &Value, name: &str) -> Option<i64> {
    let Value::Array(a) = options else { return None };
    a.get(&Key::from_string(PhpStr::from(name))).map(|v| v.deref().to_int())
}

/// php_password_make_salt(): `len` characters of the base64 of random
/// bytes, '+' made '.'.
fn make_salt(len: usize) -> Vec<u8> {
    let mut raw = vec![0u8; len * 3 / 4 + 1];
    let _ = getrandom::getrandom(&mut raw);
    base64(&raw, STD).into_iter().take(len).map(|c| if c == b'+' { b'.' } else { c }).collect()
}

const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BF: &[u8; 64] = b"./ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Base64 in `alphabet`, without padding.
fn base64(data: &[u8], alphabet: &[u8; 64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..=chunk.len() {
            out.push(alphabet[(n >> (18 - 6 * i) & 63) as usize]);
        }
    }
    out
}

/// BF_encode(): crypt_blowfish's base64 (its alphabet, its bit order).
fn bf_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i < data.len() {
        let c1 = data[i] as u32;
        i += 1;
        out.push(BF[(c1 >> 2) as usize]);
        let mut c = (c1 & 0x03) << 4;
        let Some(&c2) = data.get(i) else {
            out.push(BF[c as usize]);
            break;
        };
        i += 1;
        c |= c2 as u32 >> 4;
        out.push(BF[c as usize]);
        c = (c2 as u32 & 0x0f) << 2;
        let Some(&c3) = data.get(i) else {
            out.push(BF[c as usize]);
            break;
        };
        i += 1;
        c |= c3 as u32 >> 6;
        out.push(BF[c as usize]);
        out.push(BF[(c3 & 0x3f) as usize]);
    }
    out
}

/// BF_decode(): `size` bytes from crypt_blowfish's base64; None on a
/// character outside its alphabet.
fn bf_decode(src: &[u8], size: usize) -> Option<Vec<u8>> {
    let val = |i: usize| -> Option<u32> { BF.iter().position(|&a| Some(&a) == src.get(i)).map(|p| p as u32) };
    let mut out = Vec::with_capacity(size);
    let mut i = 0;
    while out.len() < size {
        let (c1, c2) = (val(i)?, val(i + 1)?);
        out.push(((c1 << 2) | ((c2 & 0x30) >> 4)) as u8);
        if out.len() >= size {
            break;
        }
        let c3 = val(i + 2)?;
        out.push((((c2 & 0x0f) << 4) | ((c3 & 0x3c) >> 2)) as u8);
        if out.len() >= size {
            break;
        }
        let c4 = val(i + 3)?;
        out.push((((c3 & 0x03) << 6) | c4) as u8);
        i += 4;
    }
    Some(out)
}

/// BF_set_key(): the 18 key words, and the first one's value for the
/// initial P-array (they differ only for $2a$'s countermeasure). `key` ends
/// at its first NUL, as a C string, and is used with that NUL, repeated.
fn bf_set_key(key: &[u8], bug: bool, safety: bool) -> ([u32; 18], u32) {
    let key = &key[..key.iter().position(|&c| c == 0).unwrap_or(key.len())];
    let byte = |i: usize| key.get(i).copied().unwrap_or(0);
    let mut pos = 0;
    let (mut sign, mut diff) = (0u32, 0u32);
    let mut words = [0u32; 18];
    for word in &mut words {
        let (mut correct, mut buggy) = (0u32, 0u32);
        for j in 0..4 {
            let c = byte(pos);
            correct = (correct << 8) | c as u32;
            buggy = (buggy << 8) | (c as i8 as i32 as u32);
            if j != 0 {
                sign |= buggy & 0x80;
            }
            pos = if pos >= key.len() { 0 } else { pos + 1 };
        }
        diff |= correct ^ buggy;
        *word = if bug { buggy } else { correct };
    }
    diff |= diff >> 16;
    diff &= 0xffff;
    diff = diff.wrapping_add(0xffff);
    sign <<= 9;
    sign &= !diff & if safety { 0x10000 } else { 0 };
    (words, words[0] ^ sign)
}

fn words_to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// php_crypt_blowfish_rn(): the bcrypt hash of `key` with `setting`
/// ("$2y$10$" and 22 salt characters), or None for a setting it rejects.
fn bcrypt(key: &[u8], setting: &[u8]) -> Option<Vec<u8>> {
    if setting.len() < 29 || setting[0] != b'$' || setting[1] != b'2' || setting[3] != b'$' || setting[6] != b'$' {
        return None;
    }
    // flags_by_subtype: a = countermeasure, x = the old bug, b and y = correct.
    let (bug, safety) = match setting[2] {
        b'a' => (false, true),
        b'b' | b'y' => (false, false),
        b'x' => (true, false),
        _ => return None,
    };
    if !(b'0'..=b'3').contains(&setting[4]) || !setting[5].is_ascii_digit() {
        return None;
    }
    let cost = (setting[4] - b'0') as u32 * 10 + (setting[5] - b'0') as u32;
    if !(4..=31).contains(&cost) {
        return None;
    }
    let salt = bf_decode(&setting[7..29], 16)?;
    let (words, first) = bf_set_key(key, bug, safety);
    let mut initial = words;
    initial[0] = first;
    let mut state = Blowfish::bc_init_state();
    state.salted_expand_key(&salt, &words_to_bytes(&initial));
    let key_bytes = words_to_bytes(&words);
    for _ in 0..1u64 << cost {
        state.bc_expand_key(&key_bytes);
        state.bc_expand_key(&salt);
    }
    let mut ctext = [0u32; 6];
    for (i, chunk) in b"OrpheanBeholderScryDoubt".chunks(4).enumerate() {
        ctext[i] = u32::from_be_bytes(chunk.try_into().expect("4 bytes"));
    }
    for _ in 0..64 {
        for i in (0..6).step_by(2) {
            let lr = state.bc_encrypt([ctext[i], ctext[i + 1]]);
            ctext[i] = lr[0];
            ctext[i + 1] = lr[1];
        }
    }
    let mut out = setting[..7].to_vec();
    out.extend_from_slice(&bf_encode(&salt));
    out.extend_from_slice(&bf_encode(&words_to_bytes(&ctext)[..23]));
    Some(out)
}

/// php_crypt(): bcrypt ($2y$ and kin), SHA-512 ($6$), SHA-256 ($5$) and
/// MD5 ($1$) settings; "*0" ("*1" after "*0") on failure. The traditional
/// and extended DES settings are not implemented (they fail).
fn crypt(key: &[u8], salt: &[u8]) -> Vec<u8> {
    let key = &key[..key.iter().position(|&c| c == 0).unwrap_or(key.len())];
    let made = if salt.starts_with(b"$2") {
        bcrypt(key, salt)
    } else if let Some(rest) = salt.strip_prefix(b"$6$") {
        sha_crypt::<sha2::Sha512>(key, rest, b"$6$", &SHA512_ORDER)
    } else if let Some(rest) = salt.strip_prefix(b"$5$") {
        sha_crypt::<sha2::Sha256>(key, rest, b"$5$", &SHA256_ORDER)
    } else {
        salt.strip_prefix(b"$1$").map(|rest| md5_crypt(key, rest))
    };
    match made {
        Some(h) => h,
        None if salt.starts_with(b"*0") => b"*1".to_vec(),
        None => b"*0".to_vec(),
    }
}

/// crypt(string $string, string $salt): string
fn crypt_fn(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (key, salt) = (str_arg(args, 0), str_arg(args, 1));
    Ok(Value::Str(PhpStr::new(crypt(key.as_bytes(), salt.as_bytes()))))
}

const ITOA64: &[u8; 64] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// crypt's base64: `n` characters of the 24 bits b2 b1 b0, low bits first.
fn b64_from_24bit(out: &mut Vec<u8>, b2: u8, b1: u8, b0: u8, n: usize) {
    let mut w = (u32::from(b2) << 16) | (u32::from(b1) << 8) | u32::from(b0);
    for _ in 0..n {
        out.push(ITOA64[(w & 0x3f) as usize]);
        w >>= 6;
    }
}

/// The byte triples the SHA-crypt output encodes, in order; the last is
/// written with fewer characters.
static SHA256_ORDER: [(usize, usize, usize); 11] =
    [(0, 10, 20), (21, 1, 11), (12, 22, 2), (3, 13, 23), (24, 4, 14), (15, 25, 5), (6, 16, 26), (27, 7, 17), (18, 28, 8), (9, 19, 29), (99, 31, 30)];
static SHA512_ORDER: [(usize, usize, usize); 22] = [
    (0, 21, 42), (22, 43, 1), (44, 2, 23), (3, 24, 45), (25, 46, 4), (47, 5, 26), (6, 27, 48), (28, 49, 7), (50, 8, 29), (9, 30, 51), (31, 52, 10),
    (53, 11, 32), (12, 33, 54), (34, 55, 13), (56, 14, 35), (15, 36, 57), (37, 58, 16), (59, 17, 38), (18, 39, 60), (40, 61, 19), (62, 20, 41), (99, 99, 63),
];

/// Bytes of `block` repeated to `len` bytes.
fn repeated(block: &[u8], len: usize) -> Vec<u8> {
    block.iter().copied().cycle().take(len).collect()
}

/// Ulrich Drepper's SHA-crypt ($5$ / $6$): `setting` is what follows the
/// prefix (`rounds=N$` then up to 16 salt bytes). PHP refuses rounds
/// outside 1000..=999999999.
fn sha_crypt<D: sha2::Digest>(key: &[u8], setting: &[u8], prefix: &[u8], order: &[(usize, usize, usize)]) -> Option<Vec<u8>> {
    let digest = |parts: &[&[u8]]| -> Vec<u8> {
        let mut d = D::new();
        for p in parts {
            d.update(p);
        }
        d.finalize().to_vec()
    };
    let (rounds, custom, rest) = match setting.strip_prefix(b"rounds=") {
        Some(r) => {
            let end = r.iter().position(|&c| c == b'$')?;
            let n: u64 = std::str::from_utf8(&r[..end]).ok()?.parse().ok()?;
            if !(1000..=999_999_999).contains(&n) {
                return None;
            }
            (n, true, &r[end + 1..])
        }
        None => (5000, false, setting),
    };
    let salt_end = rest.iter().position(|&c| c == b'$').unwrap_or(rest.len()).min(16);
    let salt = &rest[..salt_end];
    let b = digest(&[key, salt, key]);
    let size = b.len();
    let mut a = D::new();
    a.update(key);
    a.update(salt);
    a.update(repeated(&b, key.len()));
    let mut n = key.len();
    while n > 0 {
        a.update(if n & 1 != 0 { &b[..] } else { key });
        n >>= 1;
    }
    let a = a.finalize().to_vec();
    let p = repeated(&digest(&[&repeated(key, key.len() * key.len())]), key.len());
    let s = repeated(&digest(&[&repeated(salt, salt.len() * (16 + usize::from(a[0])))]), salt.len());
    let mut c = a;
    for i in 0..rounds {
        let mut d = D::new();
        d.update(if i & 1 != 0 { &p[..] } else { &c[..] });
        if i % 3 != 0 {
            d.update(&s);
        }
        if i % 7 != 0 {
            d.update(&p);
        }
        d.update(if i & 1 != 0 { &c[..] } else { &p[..] });
        c = d.finalize().to_vec();
    }
    let mut out = prefix.to_vec();
    if custom {
        out.extend_from_slice(format!("rounds={rounds}$").as_bytes());
    }
    out.extend_from_slice(salt);
    out.push(b'$');
    let byte = |i: usize| if i < size { c[i] } else { 0 };
    for (k, &(x, y, z)) in order.iter().enumerate() {
        let n = if k + 1 < order.len() { 4 } else if size == 64 { 2 } else { 3 };
        b64_from_24bit(&mut out, byte(x), byte(y), byte(z), n);
    }
    Some(out)
}

/// Poul-Henning Kamp's MD5-crypt ($1$): up to 8 salt bytes.
fn md5_crypt(key: &[u8], setting: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    let salt_end = setting.iter().position(|&c| c == b'$').unwrap_or(setting.len()).min(8);
    let salt = &setting[..salt_end];
    let alt = md5::Md5::new().chain_update(key).chain_update(salt).chain_update(key).finalize();
    let mut ctx = md5::Md5::new();
    ctx.update(key);
    ctx.update(b"$1$");
    ctx.update(salt);
    ctx.update(repeated(&alt, key.len()));
    let mut n = key.len();
    while n > 0 {
        ctx.update(if n & 1 != 0 { &[0u8][..] } else { &key[..1] });
        n >>= 1;
    }
    let mut fin = ctx.finalize().to_vec();
    for i in 0..1000 {
        let mut d = md5::Md5::new();
        d.update(if i & 1 != 0 { key } else { &fin[..] });
        if i % 3 != 0 {
            d.update(salt);
        }
        if i % 7 != 0 {
            d.update(key);
        }
        d.update(if i & 1 != 0 { &fin[..] } else { key });
        fin = d.finalize().to_vec();
    }
    let mut out = b"$1$".to_vec();
    out.extend_from_slice(salt);
    out.push(b'$');
    for (x, y, z) in [(0, 6, 12), (1, 7, 13), (2, 8, 14), (3, 9, 15), (4, 10, 5)] {
        b64_from_24bit(&mut out, fin[x], fin[y], fin[z], 4);
    }
    b64_from_24bit(&mut out, 0, 0, fin[11], 2);
    out
}

fn argon2_params(hash: &[u8]) -> Option<(i64, i64, i64, i64)> {
    let rest = hash.strip_prefix(b"$argon2i$").or_else(|| hash.strip_prefix(b"$argon2id$"))?;
    // sscanf("v=%ld$m=%ld,t=%ld,p=%ld"): what it reads before a mismatch.
    let text = String::from_utf8_lossy(rest);
    let mut values = [None; 4];
    let mut s = text.as_ref();
    for (i, prefix) in ["v=", "$m=", ",t=", ",p="].iter().enumerate() {
        let Some(after) = s.strip_prefix(prefix) else { break };
        let end = after.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(after.len());
        let Ok(n) = after[..end].parse::<i64>() else { break };
        values[i] = Some(n);
        s = &after[end..];
    }
    Some((values[0].unwrap_or(0), values[1].unwrap_or(0), values[2].unwrap_or(0), values[3].unwrap_or(0)))
}

fn argon2_hash(ctx: &mut dyn Ctx, password: &[u8], options: &Value, algo: Algorithm, name: &str) -> Result<Vec<u8>, Throw> {
    let memory = option(options, "memory_cost").unwrap_or(ARGON2_MEMORY_COST);
    if !(8..=(u32::MAX as i64)).contains(&memory) {
        return Err(ctx.throw("ValueError", "Memory cost is outside of allowed memory range".into()));
    }
    let time = option(options, "time_cost").unwrap_or(ARGON2_TIME_COST);
    if !(1..=(u32::MAX as i64)).contains(&time) {
        return Err(ctx.throw("ValueError", "Time cost is outside of allowed time range".into()));
    }
    let threads = option(options, "threads").unwrap_or(ARGON2_THREADS);
    if threads == 0 || threads > 0xFF_FFFF {
        return Err(ctx.throw("ValueError", "Invalid number of threads".into()));
    }
    if salt_given(options) {
        ctx.warn("password_hash(): The \"salt\" option has been ignored, since providing a custom salt is no longer supported");
    }
    let salt = make_salt(16);
    let params = Params::new(memory as u32, time as u32, threads as u32, Some(32))
        .map_err(|_| ctx.throw("ValueError", "Memory cost is too small".into()))?;
    let mut out = [0u8; 32];
    Argon2::new(algo, Version::V0x13, params)
        .hash_password_into(password, &salt, &mut out)
        .map_err(|e| ctx.throw("ValueError", e.to_string()))?;
    let mut encoded = format!("${name}$v=19$m={memory},t={time},p={threads}$").into_bytes();
    encoded.extend_from_slice(&base64(&salt, STD));
    encoded.push(b'$');
    encoded.extend_from_slice(&base64(&out, STD));
    Ok(encoded)
}

/// argon2_verify(): the hash recomputed from its own parameters and salt.
fn argon2_verify(password: &[u8], hash: &[u8], algo: Algorithm) -> bool {
    let text = String::from_utf8_lossy(hash);
    let parts: Vec<&str> = text.split('$').collect();
    // "", name, "v=19", "m=..,t=..,p=..", salt, hash
    let [_, name, version, params, salt, digest] = parts[..] else { return false };
    if name != if algo == Algorithm::Argon2i { "argon2i" } else { "argon2id" } || version != "v=19" {
        return false;
    }
    let mut m = None;
    let mut t = None;
    let mut p = None;
    for kv in params.split(',') {
        match kv.split_once('=') {
            Some(("m", v)) => m = v.parse::<u32>().ok(),
            Some(("t", v)) => t = v.parse::<u32>().ok(),
            Some(("p", v)) => p = v.parse::<u32>().ok(),
            _ => return false,
        }
    }
    let (Some(m), Some(t), Some(p), Some(salt), Some(digest)) = (m, t, p, b64_decode(salt), b64_decode(digest)) else { return false };
    let Ok(params) = Params::new(m, t, p, Some(digest.len())) else { return false };
    let mut out = vec![0u8; digest.len()];
    if Argon2::new(algo, Version::V0x13, params).hash_password_into(password, &salt, &mut out).is_err() {
        return false;
    }
    out.iter().zip(&digest).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Standard base64 without padding (argon2's encoding), strictly.
fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.bytes() {
        let v = STD.iter().position(|&a| a == c)? as u32;
        bits = (bits << 6) | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    Some(out)
}

fn salt_given(options: &Value) -> bool {
    matches!(options, Value::Array(a) if a.get(&Key::from_string(PhpStr::from("salt"))).is_some())
}

fn password_hash(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let password = str_arg(args, 0);
    let Some(algo) = algo_arg(&arg(args, 1)) else {
        return Err(ctx.throw("ValueError", "password_hash(): Argument #2 ($algo) must be a valid password hashing algorithm".into()));
    };
    let options = arg(args, 2);
    let hash = match algo {
        Algo::Bcrypt => {
            if password.as_bytes().contains(&0) {
                return Err(ctx.throw("ValueError", "Bcrypt password must not contain null character".into()));
            }
            let cost = option(&options, "cost").unwrap_or(BCRYPT_COST);
            if !(4..=31).contains(&cost) {
                return Err(ctx.throw("ValueError", format!("Invalid bcrypt cost parameter specified: {cost}")));
            }
            if salt_given(&options) {
                ctx.warn("password_hash(): The \"salt\" option has been ignored, since providing a custom salt is no longer supported");
            }
            let mut setting = format!("$2y${cost:02}$").into_bytes();
            setting.extend_from_slice(&make_salt(22));
            match bcrypt(password.as_bytes(), &setting) {
                Some(h) => h,
                None => return Err(ctx.throw("Error", "Password hashing failed for unknown reason".into())),
            }
        }
        Algo::Argon2i => argon2_hash(ctx, password.as_bytes(), &options, Algorithm::Argon2i, "argon2i")?,
        Algo::Argon2id => argon2_hash(ctx, password.as_bytes(), &options, Algorithm::Argon2id, "argon2id")?,
    };
    Ok(Value::Str(PhpStr::new(hash)))
}

fn password_verify(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let password = str_arg(args, 0);
    let hash = str_arg(args, 1);
    let (password, hash) = (password.as_bytes(), hash.as_bytes());
    // An unknown hash goes to bcrypt's check, which is crypt()'s.
    Ok(Value::Bool(match Algo::of_hash(hash).unwrap_or(Algo::Bcrypt) {
        Algo::Bcrypt => {
            let computed = crypt(password, hash);
            hash.len() >= 13 && computed.len() == hash.len() && computed.iter().zip(hash).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
        }
        Algo::Argon2i => argon2_verify(password, hash, Algorithm::Argon2i),
        Algo::Argon2id => argon2_verify(password, hash, Algorithm::Argon2id),
    }))
}

fn password_needs_rehash(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let hash = str_arg(args, 0);
    let Some(new) = algo_arg(&arg(args, 1)) else { return Ok(Value::Bool(false)) };
    let options = arg(args, 2);
    let old = Algo::of_hash(hash.as_bytes());
    if old != Some(new) {
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(match new {
        Algo::Bcrypt => bcrypt_cost(hash.as_bytes()) != option(&options, "cost").unwrap_or(BCRYPT_COST),
        Algo::Argon2i | Algo::Argon2id => {
            let (_, m, t, p) = argon2_params(hash.as_bytes()).unwrap_or_default();
            option(&options, "memory_cost").unwrap_or(ARGON2_MEMORY_COST) != m
                || option(&options, "time_cost").unwrap_or(ARGON2_TIME_COST) != t
                || option(&options, "threads").unwrap_or(ARGON2_THREADS) != p
        }
    }))
}

/// sscanf("$2y$%ld$"): the cost a bcrypt hash names.
fn bcrypt_cost(hash: &[u8]) -> i64 {
    let digits: Vec<u8> = hash[4..].iter().take_while(|c| c.is_ascii_digit()).copied().collect();
    String::from_utf8_lossy(&digits).parse().unwrap_or(BCRYPT_COST)
}

fn password_get_info(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let hash = str_arg(args, 0);
    let hash = hash.as_bytes();
    let key = |s: &str| Key::from_string(PhpStr::from(s));
    let mut info = PhpArray::new();
    let mut options = PhpArray::new();
    match Algo::of_hash(hash) {
        None => {
            info.insert(key("algo"), Value::Null);
            info.insert(key("algoName"), Value::str("unknown"));
        }
        Some(algo) => {
            info.insert(key("algo"), Value::str(algo.ident()));
            info.insert(key("algoName"), Value::str(algo.name()));
            match algo {
                Algo::Bcrypt => {
                    options.insert(key("cost"), Value::Int(bcrypt_cost(hash)));
                }
                Algo::Argon2i | Algo::Argon2id => {
                    let (_, m, t, p) = argon2_params(hash).unwrap_or((0, ARGON2_MEMORY_COST, ARGON2_TIME_COST, ARGON2_THREADS));
                    options.insert(key("memory_cost"), Value::Int(m));
                    options.insert(key("time_cost"), Value::Int(t));
                    options.insert(key("threads"), Value::Int(p));
                }
            }
        }
    }
    info.insert(key("options"), Value::Array(Rc::new(options)));
    Ok(Value::Array(Rc::new(info)))
}

fn password_algos(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let algos = [Algo::Bcrypt, Algo::Argon2i, Algo::Argon2id].map(|a| Value::str(a.ident()));
    Ok(Value::Array(Rc::new(PhpArray::from_list(algos.to_vec()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcrypt_known_hashes() {
        // From PHP 8.4: crypt('rasmuslerdorf', '$2y$07$usesomesillystringforsalt$')
        // and password_hash() outputs verified there.
        assert_eq!(
            bcrypt(b"rasmuslerdorf", b"$2y$07$usesomesillystringforsalt$").unwrap(),
            b"$2y$07$usesomesillystringfore2uDLvp1Ii2e./U9C8sBjqp8I90dH6hi".to_vec()
        );
        // OpenBSD's test vector: an empty key.
        assert_eq!(
            bcrypt(b"", b"$2a$06$DCq7YPn5Rq63x1Lad4cll.").unwrap(),
            b"$2a$06$DCq7YPn5Rq63x1Lad4cll.TV4S6ytwfsfvkgY8jIucDrjc8deX1s.".to_vec()
        );
    }

    #[test]
    fn bf_base64_round_trips() {
        let salt = bf_decode(b"usesomesillystringfore", 16).unwrap();
        assert_eq!(bf_encode(&salt), b"usesomesillystringfore".to_vec());
    }
}
