//! zlib: gzcompress and friends, deflate_/inflate_ contexts, gzip files
//! (gzopen; gzread and the rest are the file functions) and the
//! compress.zlib:// wrapper. Compression is flate2 over miniz_oxide (pure Rust, no exported symbols).
//! (Its output is the same as zlib's for most inputs and levels, not all:
//! a port of zlib's deflate would make every byte match.)
//!
//! Reference: $PHP_SRC_PATH/ext/zlib/zlib.c, zlib_fopen_wrapper.c

use crate::{arg, str_arg};
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Throw};
use p8_runtime::object::{CLASS_FINAL, CLASS_NOT_SERIALIZABLE, CLASS_UNCLONEABLE, Native, Object};
use p8_runtime::stream::{Stream, Target};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

/// ZLIB_ENCODING_*: zlib's windowBits for each format.
const RAW: i64 = -15;
const GZIP: i64 = 31;
const DEFLATE: i64 = 15;
/// Any format, detected (zlib_decode).
const ANY: i64 = 47;

pub(crate) static CONSTANTS: &[(&str, Constant)] = &[
    ("FORCE_GZIP", Constant::Int(31)),
    ("FORCE_DEFLATE", Constant::Int(15)),
    ("ZLIB_ENCODING_RAW", Constant::Int(-15)),
    ("ZLIB_ENCODING_GZIP", Constant::Int(31)),
    ("ZLIB_ENCODING_DEFLATE", Constant::Int(15)),
    ("ZLIB_NO_FLUSH", Constant::Int(0)),
    ("ZLIB_PARTIAL_FLUSH", Constant::Int(1)),
    ("ZLIB_SYNC_FLUSH", Constant::Int(2)),
    ("ZLIB_FULL_FLUSH", Constant::Int(3)),
    ("ZLIB_BLOCK", Constant::Int(5)),
    ("ZLIB_FINISH", Constant::Int(4)),
    ("ZLIB_FILTERED", Constant::Int(1)),
    ("ZLIB_HUFFMAN_ONLY", Constant::Int(2)),
    ("ZLIB_RLE", Constant::Int(3)),
    ("ZLIB_FIXED", Constant::Int(4)),
    ("ZLIB_DEFAULT_STRATEGY", Constant::Int(0)),
    ("ZLIB_VERSION", Constant::Str(b"1.3.1")),
    ("ZLIB_VERNUM", Constant::Int(4880)),
    ("ZLIB_OK", Constant::Int(0)),
    ("ZLIB_STREAM_END", Constant::Int(1)),
    ("ZLIB_NEED_DICT", Constant::Int(2)),
    ("ZLIB_ERRNO", Constant::Int(-1)),
    ("ZLIB_STREAM_ERROR", Constant::Int(-2)),
    ("ZLIB_DATA_ERROR", Constant::Int(-3)),
    ("ZLIB_MEM_ERROR", Constant::Int(-4)),
    ("ZLIB_BUF_ERROR", Constant::Int(-5)),
    ("ZLIB_VERSION_ERROR", Constant::Int(-6)),
];

pub(crate) static BUILTINS: &[Builtin] = &[
    Builtin { name: "gzcompress", run: gzcompress, by_ref: &[] },
    Builtin { name: "gzdeflate", run: gzdeflate, by_ref: &[] },
    Builtin { name: "gzencode", run: gzencode, by_ref: &[] },
    Builtin { name: "zlib_encode", run: zlib_encode, by_ref: &[] },
    Builtin { name: "gzuncompress", run: gzuncompress, by_ref: &[] },
    Builtin { name: "gzinflate", run: gzinflate, by_ref: &[] },
    Builtin { name: "gzdecode", run: gzdecode, by_ref: &[] },
    Builtin { name: "zlib_decode", run: zlib_decode, by_ref: &[] },
    Builtin { name: "deflate_init", run: deflate_init, by_ref: &[] },
    Builtin { name: "deflate_add", run: deflate_add, by_ref: &[] },
    Builtin { name: "inflate_init", run: inflate_init, by_ref: &[] },
    Builtin { name: "inflate_add", run: inflate_add, by_ref: &[] },
    Builtin { name: "inflate_get_status", run: inflate_get_status, by_ref: &[] },
    Builtin { name: "inflate_get_read_len", run: inflate_get_read_len, by_ref: &[] },
    Builtin { name: "gzopen", run: gzopen, by_ref: &[] },
    Builtin { name: "gzfile", run: gzfile, by_ref: &[] },
    Builtin { name: "readgzfile", run: readgzfile, by_ref: &[] },
    Builtin { name: "zlib_get_coding_type", run: zlib_get_coding_type, by_ref: &[] },
    Builtin { name: "ob_gzhandler", run: ob_gzhandler, by_ref: &[] },
    // The gz* file functions are the file functions.
    Builtin { name: "gzread", run: crate::file::fread, by_ref: &[] },
    Builtin { name: "gzwrite", run: crate::file::fwrite, by_ref: &[] },
    Builtin { name: "gzputs", run: crate::file::fwrite, by_ref: &[] },
    Builtin { name: "gzclose", run: crate::file::fclose, by_ref: &[] },
    Builtin { name: "gzeof", run: crate::file::feof, by_ref: &[] },
    Builtin { name: "gztell", run: crate::file::ftell, by_ref: &[] },
    Builtin { name: "gzseek", run: crate::file::fseek, by_ref: &[] },
    Builtin { name: "gzrewind", run: crate::file::rewind, by_ref: &[] },
    Builtin { name: "gzgets", run: crate::file::fgets, by_ref: &[] },
    Builtin { name: "gzgetc", run: crate::file::fgetc, by_ref: &[] },
    Builtin { name: "gzpassthru", run: crate::file::fpassthru, by_ref: &[] },
];

/// zError: zlib's text for a status.
fn z_error(status: i32) -> &'static str {
    match status {
        2 => "need dictionary",
        1 => "stream end",
        0 => "",
        -1 => "file error",
        -2 => "stream error",
        -3 => "data error",
        -4 => "insufficient memory",
        -5 => "buffer error",
        -6 => "incompatible version",
        _ => "",
    }
}

const Z_DATA_ERROR: i32 = -3;
const Z_MEM_ERROR: i32 = -4;
const Z_BUF_ERROR: i32 = -5;

fn compression(level: i64) -> Compression {
    if level < 0 { Compression::default() } else { Compression::new(level as u32) }
}

/// The gzip header zlib writes (no name, time 0, unix), its XFL from the
/// level as deflate.c sets it.
fn gzip_header(level: i64) -> [u8; 10] {
    let level = if level < 0 { 6 } else { level };
    let xfl = if level == 9 { 2 } else if level < 2 { 4 } else { 0 };
    [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, xfl, 3]
}

/// The zlib header deflate.c writes: 32K window, FLEVEL from the level.
fn zlib_header(level: i64) -> [u8; 2] {
    let level = if level < 0 { 6 } else { level };
    let flevel: u16 = match level {
        0 | 1 => 0,
        2..=5 => 1,
        6 => 2,
        _ => 3,
    };
    let mut h: u16 = (0x78 << 8) | (flevel << 6);
    h += 31 - h % 31;
    h.to_be_bytes()
}

/// Adler-32 (zlib's checksum), continued from `a`.
fn adler32(mut a: u32, data: &[u8]) -> u32 {
    let (mut s1, mut s2) = (a & 0xffff, a >> 16);
    for chunk in data.chunks(5552) {
        for &b in chunk {
            s1 += u32::from(b);
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
    }
    a = (s2 << 16) | s1;
    a
}

/// Compress all of `data` in format `encoding` (php_zlib_encode).
pub(crate) fn encode(data: &[u8], encoding: i64, level: i64) -> Vec<u8> {
    let raw = |zlib: bool| {
        use std::io::Write;
        let mut e = flate2::write::DeflateEncoder::new(Vec::with_capacity(data.len() / 2 + 64), compression(level));
        let _ = e.write_all(data);
        let body = e.finish().unwrap_or_default();
        if !zlib {
            return body;
        }
        // The header and trailer as zlib writes them, around the raw data.
        let mut out = zlib_header(level).to_vec();
        out.extend_from_slice(&body);
        out.extend_from_slice(&adler32(1, data).to_be_bytes());
        out
    };
    match encoding {
        GZIP => {
            let mut out = gzip_header(level).to_vec();
            out.extend_from_slice(&raw(false));
            let mut crc = flate2::Crc::new();
            crc.update(data);
            out.extend_from_slice(&crc.sum().to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out
        }
        DEFLATE => raw(true),
        _ => raw(false),
    }
}

/// Inflate `data` (raw, or with a zlib header when `zlib`) to its end:
/// the bytes, or zlib's status. `max` (0: none) caps the output.
fn inflate_all(data: &[u8], zlib: bool, max: usize) -> Result<(Vec<u8>, usize), i32> {
    let mut d = Decompress::new(zlib);
    let mut out = Vec::with_capacity(data.len().max(64));
    // PHP hands zlib the string's terminating NUL too.
    let mut input = data.to_vec();
    input.push(0);
    loop {
        let consumed = d.total_in() as usize;
        if max > 0 && out.len() >= max {
            return Err(Z_MEM_ERROR);
        }
        let room = if max > 0 { (max - out.len()).min(out.capacity() - out.len()).max(1) } else { out.capacity() - out.len() };
        let mut chunk = Vec::with_capacity(room.max(64).min(if max > 0 { max - out.len() } else { usize::MAX }));
        match d.decompress_vec(&input[consumed..], &mut chunk, FlushDecompress::None) {
            Ok(Status::StreamEnd) => {
                out.extend_from_slice(&chunk);
                if max > 0 && out.len() > max {
                    return Err(Z_MEM_ERROR);
                }
                return Ok((out, d.total_in() as usize));
            }
            Ok(_) => {
                let progressed = d.total_in() as usize != consumed || !chunk.is_empty();
                out.extend_from_slice(&chunk);
                if !progressed {
                    return Err(if consumed >= input.len() { Z_DATA_ERROR } else { Z_BUF_ERROR });
                }
                if d.total_in() as usize >= input.len() && chunk.is_empty() {
                    return Err(Z_DATA_ERROR);
                }
            }
            Err(_) => return Err(Z_DATA_ERROR),
        }
    }
}

/// A gzip member: its deflate body, after the header (None: not gzip).
fn gzip_body(data: &[u8]) -> Option<&[u8]> {
    if data.len() < 10 || data[0] != 0x1f || data[1] != 0x8b || data[2] != 8 {
        return None;
    }
    let flags = data[3];
    let mut at = 10;
    if flags & 4 != 0 {
        let xlen = usize::from(*data.get(at)?) | usize::from(*data.get(at + 1)?) << 8;
        at += 2 + xlen;
    }
    for bit in [8, 16] {
        if flags & bit != 0 {
            at += data.get(at..)?.iter().position(|&c| c == 0)? + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    data.get(at..)
}

/// Decode `data` in format `encoding` (php_zlib_decode): the bytes, or
/// zlib's status for the warning.
pub(crate) fn decode(data: &[u8], encoding: i64, max: usize) -> Result<Vec<u8>, i32> {
    if data.is_empty() {
        return Err(Z_DATA_ERROR);
    }
    let gzip = || gzip_body(data).ok_or(Z_DATA_ERROR).and_then(|body| inflate_all(body, false, max).map(|(o, _)| o));
    match encoding {
        GZIP => gzip(),
        DEFLATE => inflate_all(data, true, max).map(|(o, _)| o),
        RAW => inflate_all(data, false, max).map(|(o, _)| o),
        _ => {
            // Any: gzip or zlib by their headers, else (a data error) raw.
            let r = if data.starts_with(&[0x1f, 0x8b]) { gzip() } else { inflate_all(data, true, max).map(|(o, _)| o) };
            match r {
                Err(Z_DATA_ERROR) => inflate_all(data, false, max).map(|(o, _)| o),
                r => r,
            }
        }
    }
}

/// gzcompress & co: (data, level, encoding) or zlib_encode's (data, encoding, level).
fn encode_func(ctx: &mut dyn Ctx, args: &[Value], func: &str, default_encoding: Option<i64>) -> Result<Value, Throw> {
    let data = str_arg(args, 0);
    let (level, encoding, level_arg, encoding_arg) = match default_encoding {
        Some(e) => (args.get(1).map_or(-1, |v| v.deref().to_int()), args.get(2).map_or(e, |v| v.deref().to_int()), 2, 3),
        None => (args.get(2).map_or(-1, |v| v.deref().to_int()), arg(args, 1).to_int(), 3, 2),
    };
    if !(-1..=9).contains(&level) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #{level_arg} ($level) must be between -1 and 9")));
    }
    if !matches!(encoding, RAW | GZIP | DEFLATE) {
        return Err(ctx.throw(
            "ValueError",
            format!("{func}(): Argument #{encoding_arg} ($encoding) must be one of ZLIB_ENCODING_RAW, ZLIB_ENCODING_GZIP, or ZLIB_ENCODING_DEFLATE"),
        ));
    }
    Ok(Value::str(PhpStr::new(encode(data.as_bytes(), encoding, level))))
}

fn gzcompress(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    encode_func(ctx, args, "gzcompress", Some(DEFLATE))
}

fn gzdeflate(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    encode_func(ctx, args, "gzdeflate", Some(RAW))
}

fn gzencode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    encode_func(ctx, args, "gzencode", Some(GZIP))
}

fn zlib_encode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    encode_func(ctx, args, "zlib_encode", None)
}

fn decode_func(ctx: &mut dyn Ctx, args: &[Value], func: &str, encoding: i64) -> Result<Value, Throw> {
    let data = str_arg(args, 0);
    let max = args.get(1).map_or(0, |v| v.deref().to_int());
    if max < 0 {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #2 ($max_length) must be greater than or equal to 0")));
    }
    match decode(data.as_bytes(), encoding, max as usize) {
        Ok(out) => Ok(Value::str(PhpStr::new(out))),
        Err(status) => {
            ctx.warn(&format!("{func}(): {}", z_error(status)));
            Ok(Value::FALSE)
        }
    }
}

fn gzuncompress(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    decode_func(ctx, args, "gzuncompress", DEFLATE)
}

fn gzinflate(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    decode_func(ctx, args, "gzinflate", RAW)
}

fn gzdecode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    decode_func(ctx, args, "gzdecode", GZIP)
}

fn zlib_decode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    decode_func(ctx, args, "zlib_decode", ANY)
}

// --- incremental contexts --------------------------------------------------

struct Deflater {
    c: Compress,
    encoding: i64,
    level: i64,
    /// The header is out; the CRC (gzip) or Adler-32 (zlib) of what went in.
    started: bool,
    crc: flate2::Crc,
    adler: u32,
}

struct Inflater {
    d: Decompress,
    /// For gzip/any: input held until the header is complete.
    pending: Vec<u8>,
    header_done: bool,
    status: i64,
    read_len: i64,
    done: bool,
}

#[derive(Default)]
pub(crate) struct DeflateState(RefCell<Option<Deflater>>);
#[derive(Default)]
pub(crate) struct InflateState(RefCell<Option<Inflater>>);

pub(crate) fn make_deflate() -> Rc<dyn std::any::Any> {
    Rc::new(DeflateState::default())
}

pub(crate) fn make_inflate() -> Rc<dyn std::any::Any> {
    Rc::new(InflateState::default())
}

fn option(options: &Value, name: &str) -> Option<i64> {
    options.deref().as_array().and_then(|a| a.get(&p8_runtime::array::Key::from_string(PhpStr::from(name))).map(|v| v.deref().to_int()))
}

fn deflate_init(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let encoding = arg(args, 0).to_int();
    let options = arg(args, 1);
    let level = option(&options, "level").unwrap_or(-1);
    if !(-1..=9).contains(&level) {
        return Err(ctx.throw("ValueError", "deflate_init(): \"level\" option must be between -1 and 9".into()));
    }
    if let Some(m) = option(&options, "memory")
        && !(1..=9).contains(&m)
    {
        return Err(ctx.throw("ValueError", "deflate_init(): \"memory\" option must be between 1 and 9".into()));
    }
    if let Some(w) = option(&options, "window")
        && !(8..=15).contains(&w)
    {
        return Err(ctx.throw("ValueError", "deflate_init(): \"window\" option must be between 8 and 15".into()));
    }
    if let Some(s) = option(&options, "strategy")
        && !(0..=4).contains(&s)
    {
        return Err(ctx.throw(
            "ValueError",
            "deflate_init(): \"strategy\" option must be one of ZLIB_FILTERED, ZLIB_HUFFMAN_ONLY, ZLIB_RLE, ZLIB_FIXED, or ZLIB_DEFAULT_STRATEGY".into(),
        ));
    }
    if !matches!(encoding, RAW | GZIP | DEFLATE) {
        return Err(ctx.throw("ValueError", "deflate_init(): Argument #1 ($encoding) must be one of ZLIB_ENCODING_RAW, ZLIB_ENCODING_GZIP, or ZLIB_ENCODING_DEFLATE".into()));
    }
    let class = ctx.class(b"DeflateContext").expect("DeflateContext is declared");
    let o = ctx.new_object(&class);
    let state = o.native::<DeflateState>().expect("a DeflateContext");
    *state.0.borrow_mut() = Some(Deflater { c: Compress::new(compression(level), false), encoding, level, started: false, crc: flate2::Crc::new(), adler: 1 });
    Ok(Value::object(o))
}

/// A ZLIB_* flush mode.
fn flush_mode(ctx: &mut dyn Ctx, func: &str, mode: i64) -> Result<i64, Throw> {
    if !matches!(mode, 0..=5) {
        return Err(ctx.throw(
            "ValueError",
            format!("{func}(): Argument #3 ($flush_mode) must be one of ZLIB_NO_FLUSH, ZLIB_PARTIAL_FLUSH, ZLIB_SYNC_FLUSH, ZLIB_FULL_FLUSH, ZLIB_BLOCK, or ZLIB_FINISH"),
        ));
    }
    Ok(mode)
}

fn context_arg(ctx: &mut dyn Ctx, args: &[Value], func: &str, class: &str) -> Result<Rc<Object>, Throw> {
    match args.first().map(Value::deref) {
        Some(v) if v.as_object().is_some_and(|o| o.class.name.as_bytes() == class.as_bytes()) => Ok(v.cloned_object().expect("an object")),
        v => {
            let given = match &v {
                Some(v) => match v.as_object() {
                    Some(o) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                    None => v.type_name().into_owned(),
                },
                None => "null".into(),
            };
            Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($context) must be of type {class}, {given} given")))
        }
    }
}

fn deflate_add(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context_arg(ctx, args, "deflate_add", "DeflateContext")?;
    let data = str_arg(args, 1);
    let mode = flush_mode(ctx, "deflate_add", args.get(2).map_or(2, |v| v.deref().to_int()))?;
    let state = o.native::<DeflateState>().expect("a DeflateContext");
    let mut guard = state.0.borrow_mut();
    let d = guard.as_mut().expect("deflate_init made it");
    let flush = match mode {
        0 => FlushCompress::None,
        1 => FlushCompress::Partial,
        2 | 5 => FlushCompress::Sync,
        3 => FlushCompress::Full,
        _ => FlushCompress::Finish,
    };
    let mut out = Vec::new();
    if !d.started {
        match d.encoding {
            GZIP => out.extend_from_slice(&gzip_header(d.level)),
            DEFLATE => out.extend_from_slice(&zlib_header(d.level)),
            _ => {}
        }
    }
    d.started = true;
    d.crc.update(data.as_bytes());
    d.adler = adler32(d.adler, data.as_bytes());
    let bytes = data.as_bytes();
    let start = d.c.total_in();
    loop {
        let consumed = (d.c.total_in() - start) as usize;
        out.reserve(bytes.len() / 2 + 128);
        let status = d.c.compress_vec(&bytes[consumed..], &mut out, flush).unwrap_or(Status::StreamEnd);
        let done_input = (d.c.total_in() - start) as usize >= bytes.len();
        if status == Status::StreamEnd || (done_input && out.capacity() > out.len()) {
            break;
        }
    }
    if mode == 4 {
        match d.encoding {
            GZIP => {
                out.extend_from_slice(&d.crc.sum().to_le_bytes());
                out.extend_from_slice(&(d.crc.amount()).to_le_bytes());
            }
            DEFLATE => out.extend_from_slice(&d.adler.to_be_bytes()),
            _ => {}
        }
        // A finished context starts over.
        *d = Deflater { c: Compress::new(compression(d.level), false), encoding: d.encoding, level: d.level, started: false, crc: flate2::Crc::new(), adler: 1 };
    }
    Ok(Value::str(PhpStr::new(out)))
}

fn inflate_init(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let encoding = arg(args, 0).to_int();
    let options = arg(args, 1);
    if let Some(w) = option(&options, "window")
        && !(8..=15).contains(&w)
    {
        return Err(ctx.throw("ValueError", format!("zlib window size (logarithm) ({w}) must be within 8..15")));
    }
    if !matches!(encoding, RAW | GZIP | DEFLATE) {
        return Err(ctx.throw("ValueError", "Encoding mode must be ZLIB_ENCODING_RAW, ZLIB_ENCODING_GZIP or ZLIB_ENCODING_DEFLATE".into()));
    }
    let class = ctx.class(b"InflateContext").expect("InflateContext is declared");
    let o = ctx.new_object(&class);
    let state = o.native::<InflateState>().expect("an InflateContext");
    *state.0.borrow_mut() = Some(Inflater {
        d: Decompress::new(encoding == DEFLATE),
        pending: Vec::new(),
        header_done: encoding != GZIP,
        status: 0,
        read_len: 0,
        done: false,
    });
    Ok(Value::object(o))
}

fn inflate_add(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context_arg(ctx, args, "inflate_add", "InflateContext")?;
    let data = str_arg(args, 1);
    let mode = flush_mode(ctx, "inflate_add", args.get(2).map_or(2, |v| v.deref().to_int()))?;
    let state = o.native::<InflateState>().expect("an InflateContext");
    let mut guard = state.0.borrow_mut();
    let inf = guard.as_mut().expect("inflate_init made it");
    if inf.done {
        // After the end, more input starts nothing (zlib's Z_STREAM_END stays).
        inf.read_len += data.len() as i64;
        return Ok(Value::str(""));
    }
    inf.pending.extend_from_slice(data.as_bytes());
    if !inf.header_done {
        match gzip_body(&inf.pending) {
            Some(body) => {
                let skip = inf.pending.len() - body.len();
                inf.pending.drain(..skip);
                inf.header_done = true;
            }
            None if inf.pending.len() < 10 && mode != 4 => return Ok(Value::str("")),
            None => {
                drop(guard);
                ctx.warn("inflate_add(): data error");
                return Ok(Value::FALSE);
            }
        }
    }
    let input = std::mem::take(&mut inf.pending);
    let mut out = Vec::new();
    let start = inf.d.total_in();
    loop {
        let consumed = (inf.d.total_in() - start) as usize;
        out.reserve(input.len() * 2 + 256);
        match inf.d.decompress_vec(&input[consumed..], &mut out, FlushDecompress::None) {
            Ok(Status::StreamEnd) => {
                inf.done = true;
                inf.status = 1;
                break;
            }
            Ok(_) => {
                let used = (inf.d.total_in() - start) as usize;
                if used >= input.len() && out.len() < out.capacity() {
                    inf.status = 0;
                    break;
                }
                if used == consumed && out.len() < out.capacity() {
                    break;
                }
            }
            Err(_) => {
                drop(guard);
                ctx.warn("inflate_add(): data error");
                return Ok(Value::FALSE);
            }
        }
    }
    inf.read_len = inf.d.total_in() as i64;
    if mode == 4 && !inf.done {
        inf.status = -5;
    }
    Ok(Value::str(PhpStr::new(out)))
}

fn inflate_get_status(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context_arg(ctx, args, "inflate_get_status", "InflateContext")?;
    let state = o.native::<InflateState>().expect("an InflateContext");
    Ok(Value::int(state.0.borrow().as_ref().map_or(0, |i| i.status)))
}

fn inflate_get_read_len(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let o = context_arg(ctx, args, "inflate_get_read_len", "InflateContext")?;
    let state = o.native::<InflateState>().expect("an InflateContext");
    Ok(Value::int(state.0.borrow().as_ref().map_or(0, |i| i.read_len)))
}

fn no_construct(class: &'static str, func: &'static str) -> impl Fn(&mut dyn Ctx) -> Throw {
    move |ctx| ctx.throw("Error", format!("Cannot directly construct {class}, use {func}() instead"))
}

fn deflate_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(no_construct("DeflateContext", "deflate_init")(ctx))
}

fn inflate_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Err(no_construct("InflateContext", "inflate_init")(ctx))
}

pub(crate) static CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "InflateContext",
        parent: None,
        interfaces: &[],
        flags: CLASS_FINAL | CLASS_UNCLONEABLE | CLASS_NOT_SERIALIZABLE,
        properties: &[],
        methods: &[("__construct", Some(Native::Method(inflate_construct)))],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "DeflateContext",
        parent: None,
        interfaces: &[],
        flags: CLASS_FINAL | CLASS_UNCLONEABLE | CLASS_NOT_SERIALIZABLE,
        properties: &[],
        methods: &[("__construct", Some(Native::Method(deflate_construct)))],
        constants: &[],
        debug_info: None,
    },
];

// --- gzip files -------------------------------------------------------------

/// Every gzip member of `data`, inflated and joined; a file that is not
/// gzip reads as it is (zlib's gzread is transparent).
fn gunzip_file(data: &[u8]) -> Vec<u8> {
    if gzip_body(data).is_none() {
        return data.to_vec();
    }
    let mut out = Vec::new();
    let mut rest = data;
    while let Some(body) = gzip_body(rest) {
        let Ok((text, used)) = inflate_all(body, false, 0) else { break };
        out.extend_from_slice(&text);
        // The NUL inflate_all appends is not the file's.
        let used = used.min(body.len());
        let after = &body[used..];
        rest = if after.len() >= 8 { &after[8..] } else { &[] };
        if rest.is_empty() {
            break;
        }
    }
    out
}

/// A gzip file opened as a stream ("r", "w9", "a", ...): read whole and
/// inflated, or written whole and compressed when it closes.
pub(crate) fn open_gz(ctx: &mut dyn Ctx, func: &str, path: &str, mode: &str) -> Option<Stream> {
    if mode.contains('+') {
        ctx.warn(&format!("{func}(): Cannot open a zlib stream for reading and writing at the same time!"));
        return None;
    }
    let kind = mode.chars().next().unwrap_or('r');
    if !matches!(kind, 'r' | 'w' | 'a' | 'x' | 'c') {
        ctx.warn(&format!("{func}({path}): Failed to open stream: Invalid argument"));
        return None;
    }
    let os = p8_runtime::cwd::resolve(ctx, path);
    if kind == 'r' {
        match std::fs::read(&os) {
            Ok(bytes) => {
                let mut s = Stream::new(Target::Memory(gunzip_file(&bytes)), path, "rb");
                s.zlib = true;
                Some(s)
            }
            Err(e) => {
                ctx.warn(&format!("{func}({path}): Failed to open stream: {}", crate::file::os_error(&e)));
                None
            }
        }
    } else {
        let level = mode.chars().find(|c| c.is_ascii_digit()).map_or(-1, |c| i64::from(c as u8 - b'0'));
        let append = kind == 'a';
        let file = std::fs::OpenOptions::new().write(true).create(true).append(append).truncate(!append).open(&os);
        match file {
            Ok(mut f) => {
                let mut s = Stream::new(Target::Memory(Vec::new()), path, "wb");
                s.zlib = true;
                s.on_close = Some(Box::new(move |data: &[u8]| {
                    use std::io::Write;
                    let _ = f.write_all(&encode(data, GZIP, level));
                }));
                Some(s)
            }
            Err(e) => {
                ctx.warn(&format!("{func}({path}): Failed to open stream: {}", crate::file::os_error(&e)));
                None
            }
        }
    }
}

fn path_arg(args: &[Value], i: usize) -> String {
    String::from_utf8_lossy(str_arg(args, i).as_bytes()).into_owned()
}

fn gzopen(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = path_arg(args, 0);
    let mode = path_arg(args, 1);
    let path = path.strip_prefix("compress.zlib://").unwrap_or(&path).to_string();
    match open_gz(ctx, "gzopen", &path, &mode) {
        Some(s) => Ok(crate::file::new_stream(ctx, s)),
        None => Ok(Value::FALSE),
    }
}

/// The inflated text of a gzip (or plain) file, with gzopen's warning.
fn read_gz(ctx: &mut dyn Ctx, func: &str, path: &str) -> Option<Vec<u8>> {
    match std::fs::read(p8_runtime::cwd::resolve(ctx, path)) {
        Ok(bytes) => Some(gunzip_file(&bytes)),
        Err(e) => {
            ctx.warn(&format!("{func}({path}): Failed to open stream: {}", crate::file::os_error(&e)));
            None
        }
    }
}

fn gzfile(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = path_arg(args, 0);
    let Some(text) = read_gz(ctx, "gzfile", &path) else { return Ok(Value::FALSE) };
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, &c) in text.iter().enumerate() {
        if c == b'\n' {
            lines.push(Value::str(PhpStr::from(&text[start..=i])));
            start = i + 1;
        }
    }
    if start < text.len() {
        lines.push(Value::str(PhpStr::from(&text[start..])));
    }
    Ok(Value::array(p8_runtime::array::PhpArray::from_list(lines)))
}

fn readgzfile(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = path_arg(args, 0);
    let Some(text) = read_gz(ctx, "readgzfile", &path) else { return Ok(Value::FALSE) };
    ctx.echo(&text);
    Ok(Value::int(text.len() as i64))
}

fn zlib_get_coding_type(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::FALSE)
}

/// ob_gzhandler(string $data, int $flags): without an Accept-Encoding
/// header (the CLI) the output passes as it is.
fn ob_gzhandler(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::FALSE)
}
