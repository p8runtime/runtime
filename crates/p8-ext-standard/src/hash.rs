//! Hashes: md5, sha1, crc32, hash() and hash_hmac() with the algorithms
//! scripts use, hash_equals, hash_algos, and the _file forms.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/md5.c, sha1.c, crc32.c,
//! $PHP_SRC_PATH/ext/hash/hash.c

use crate::{arg, str_arg};
use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Builtin, Ctx, Throw};
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
    Builtin { name: "hash_hmac_algos", run: hash_algos, by_ref: &[] },
    Builtin { name: "hash_equals", run: hash_equals, by_ref: &[] },
];

const ALGOS: &[&str] =
    &["md5", "sha1", "sha224", "sha256", "sha384", "sha512/224", "sha512/256", "sha512", "adler32", "crc32b", "fnv132", "fnv1a32", "fnv164", "fnv1a64", "joaat"];

/// Algorithm `name`'s digest of `data`, or None for one not known.
fn digest(name: &str, data: &[u8]) -> Option<Vec<u8>> {
    use sha2::Digest;
    Some(match name {
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
        _ => 64,
    }
}

fn hex(bytes: &[u8]) -> Vec<u8> {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes.iter().flat_map(|b| [DIGITS[(b >> 4) as usize], DIGITS[(b & 15) as usize]]).collect()
}

/// The digest as a binary string, or its hex (`binary` false).
fn shown(d: Vec<u8>, binary: bool) -> Value {
    Value::Str(PhpStr::new(if binary { d } else { hex(&d) }))
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
    Ok(Value::Int(i64::from(crc32fast::hash(str_arg(args, 0).as_bytes()))))
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
    let Some(b) = read_file(ctx, "md5_file", &str_arg(args, 0)) else { return Ok(Value::Bool(false)) };
    Ok(shown(digest("md5", &b).expect("md5"), binary(args, 1)))
}

fn sha1_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(b) = read_file(ctx, "sha1_file", &str_arg(args, 0)) else { return Ok(Value::Bool(false)) };
    Ok(shown(digest("sha1", &b).expect("sha1"), binary(args, 1)))
}

fn algo(ctx: &mut dyn Ctx, func: &str, args: &[Value]) -> Result<String, Throw> {
    let name = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).to_ascii_lowercase();
    if !ALGOS.contains(&name.as_str()) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($algo) must be a valid hashing algorithm")));
    }
    Ok(name)
}

/// hash(string $algo, string $data, bool $binary = false)
fn hash(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = algo(ctx, "hash", args)?;
    Ok(shown(digest(&name, str_arg(args, 1).as_bytes()).expect("a known algorithm"), binary(args, 2)))
}

fn hash_file(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = algo(ctx, "hash_file", args)?;
    let Some(b) = read_file(ctx, "hash_file", &str_arg(args, 1)) else { return Ok(Value::Bool(false)) };
    Ok(shown(digest(&name, &b).expect("a known algorithm"), binary(args, 2)))
}

/// hash_hmac(string $algo, string $data, string $key, bool $binary = false)
fn hash_hmac(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = algo(ctx, "hash_hmac", args)?;
    if matches!(name.as_str(), "adler32" | "crc32b" | "fnv132" | "fnv1a32" | "fnv164" | "fnv1a64" | "joaat") {
        return Err(ctx.throw("ValueError", "hash_hmac(): Argument #1 ($algo) must be a valid cryptographic hashing algorithm".into()));
    }
    let data = str_arg(args, 1);
    let key = str_arg(args, 2);
    let block = block_size(&name);
    let mut k = key.as_bytes().to_vec();
    if k.len() > block {
        k = digest(&name, &k).expect("a known algorithm");
    }
    k.resize(block, 0);
    let inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).chain(data.as_bytes().iter().copied()).collect();
    let inner = digest(&name, &inner).expect("a known algorithm");
    let outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).chain(inner).collect();
    Ok(shown(digest(&name, &outer).expect("a known algorithm"), binary(args, 3)))
}

fn hash_algos(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Array(Rc::new(PhpArray::from_list(ALGOS.iter().map(|a| Value::str(*a)).collect()))))
}

/// hash_equals(string $known_string, string $user_string): compared in constant time.
fn hash_equals(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    for (i, name) in [(0, "known_string"), (1, "user_string")] {
        let v = arg(args, i);
        if !matches!(v, Value::Str(_)) {
            return Err(ctx.throw(
                "TypeError",
                format!("hash_equals(): Argument #{} (${name}) must be of type string, {} given", i + 1, v.type_name()),
            ));
        }
    }
    let (a, b) = (str_arg(args, 0), str_arg(args, 1));
    if a.len() != b.len() {
        return Ok(Value::Bool(false));
    }
    let diff = a.as_bytes().iter().zip(b.as_bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    Ok(Value::Bool(diff == 0))
}
