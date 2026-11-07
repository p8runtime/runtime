//! getimagesize, getimagesizefromstring, image_type_to_mime_type,
//! image_type_to_extension: an image's type and size from its header.
//!
//! Each format's reader works as PHP's does on a stream (reads and seeks
//! from where the type check left off), here over the file's bytes: GIF,
//! JPEG (and its APP markers), PNG, SWF (and zlib-compressed SWC), PSD, BMP,
//! TIFF, WBMP, XBM, ICO, WebP. IFF, JPEG 2000 and AVIF are recognised but
//! not read yet: getimagesize() is false for them.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/image.c

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "getimagesize", run: getimagesize, by_ref: &[1] },
    Builtin { name: "getimagesizefromstring", run: getimagesizefromstring, by_ref: &[1] },
    Builtin { name: "image_type_to_mime_type", run: image_type_to_mime_type, by_ref: &[] },
    Builtin { name: "image_type_to_extension", run: image_type_to_extension, by_ref: &[] },
];

const GIF: i64 = 1;
const JPEG: i64 = 2;
const PNG: i64 = 3;
const SWF: i64 = 4;
const PSD: i64 = 5;
const BMP: i64 = 6;
const TIFF_II: i64 = 7;
const TIFF_MM: i64 = 8;
const JPC: i64 = 9;
const JP2: i64 = 10;
const JPX: i64 = 11;
const JB2: i64 = 12;
const SWC: i64 = 13;
const IFF: i64 = 14;
const WBMP: i64 = 15;
const XBM: i64 = 16;
const ICO: i64 = 17;
const WEBP: i64 = 18;
const AVIF: i64 = 19;

/// The bytes being read and where: PHP's stream reads, getc and seeks.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Exactly `n` bytes, or None (and nothing consumed) at the end.
    fn read(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let bytes = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(bytes)
    }

    fn getc(&mut self) -> Option<u8> {
        let c = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(c)
    }

    /// A seek from here (a file can be sought past its end; not before its start).
    fn skip(&mut self, by: i64) -> bool {
        match (self.pos as i64).checked_add(by) {
            Some(p) if p >= 0 => {
                self.pos = p as usize;
                true
            }
            _ => false,
        }
    }

    fn read2(&mut self) -> u32 {
        self.read(2).map_or(0, |b| (u32::from(b[0]) << 8) | u32::from(b[1]))
    }
}

#[derive(Default)]
struct Info {
    width: u32,
    height: u32,
    bits: u32,
    channels: u32,
}

/// php_getimagetype: the type from the first bytes (a notice when there
/// are too few to tell).
fn image_type(ctx: &mut dyn Ctx, func: &str, c: &mut Cursor, input: &str) -> i64 {
    let short = |ctx: &mut dyn Ctx| ctx.notice(&format!("{func}(): Error reading from {input}!"));
    let Some(head) = c.read(3) else {
        short(ctx);
        return 0;
    };
    match head {
        b"GIF" => return GIF,
        [0xff, 0xd8, 0xff] => return JPEG,
        [0x89, b'P', b'N'] => {
            let Some(rest) = c.read(5) else {
                short(ctx);
                return 0;
            };
            if rest == [b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
                return PNG;
            }
            ctx.warn(&format!("{func}(): PNG file corrupted by ASCII conversion"));
            return 0;
        }
        b"FWS" => return SWF,
        b"CWS" => return SWC,
        b"8BP" => return PSD,
        [b'B', b'M', _] => return BMP,
        [0xff, 0x4f, 0xff] => return JPC,
        b"RIF" => {
            let Some(rest) = c.read(9) else {
                short(ctx);
                return 0;
            };
            return if &rest[5..9] == b"WEBP" { WEBP } else { 0 };
        }
        _ => {}
    }
    let Some(fourth) = c.getc() else {
        short(ctx);
        return 0;
    };
    match [head[0], head[1], head[2], fourth] {
        [b'I', b'I', 0x2a, 0x00] => return TIFF_II,
        [b'M', b'M', 0x00, 0x2a] => return TIFF_MM,
        [b'F', b'O', b'R', b'M'] => return IFF,
        [0x00, 0x00, 0x01, 0x00] => return ICO,
        _ => {}
    }
    let twelve = c.read(8).is_some();
    if twelve && c.data[..12] == [0x00, 0x00, 0x00, 0x0c, b'j', b'P', b' ', b' ', 0x0d, 0x0a, 0x87, 0x0a] {
        return JP2;
    }
    if is_avif(c.data) {
        return AVIF;
    }
    if wbmp(c.data).is_some() {
        return WBMP;
    }
    if !twelve {
        short(ctx);
        return 0;
    }
    if xbm(c.data).is_some() {
        return XBM;
    }
    0
}

/// An ISO-BMFF "ftyp" box naming the avif or avis brand.
fn is_avif(data: &[u8]) -> bool {
    data.len() >= 12 && &data[4..8] == b"ftyp" && (&data[8..12] == b"avif" || &data[8..12] == b"avis")
}

fn gif(c: &mut Cursor) -> Option<Info> {
    c.skip(3).then_some(())?;
    let d = c.read(5)?;
    Some(Info {
        width: u32::from(d[0]) | (u32::from(d[1]) << 8),
        height: u32::from(d[2]) | (u32::from(d[3]) << 8),
        bits: if d[4] & 0x80 != 0 { u32::from(d[4] & 0x07) + 1 } else { 0 },
        channels: 3,
    })
}

fn be32(b: &[u8]) -> u32 {
    (u32::from(b[0]) << 24) | (u32::from(b[1]) << 16) | (u32::from(b[2]) << 8) | u32::from(b[3])
}

fn le32(b: &[u8]) -> u32 {
    (u32::from(b[3]) << 24) | (u32::from(b[2]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[0])
}

/// php_swf_get_bits: `count` bits from bit `pos`, most significant first.
fn swf_bits(b: &[u8], pos: u64, count: u64) -> u64 {
    (pos..pos + count).fold(0, |r, i| {
        let bit = (u64::from(*b.get((i / 8) as usize).unwrap_or(&0)) >> (7 - (i % 8))) & 1;
        r + (bit << (count - (i - pos) - 1))
    })
}

/// A SWF header's frame rectangle (in twips): the width and height.
fn swf_rect(a: &[u8]) -> Info {
    let bits = swf_bits(a, 0, 5);
    let w = swf_bits(a, 5 + bits, bits).wrapping_sub(swf_bits(a, 5, bits)) / 20;
    let h = swf_bits(a, 5 + 3 * bits, bits).wrapping_sub(swf_bits(a, 5 + 2 * bits, bits)) / 20;
    Info { width: w as u32, height: h as u32, ..Info::default() }
}

/// php_handle_swc: the frame rectangle of a SWF whose body (from byte 8) is zlib data.
fn swc(data: &[u8]) -> Option<Info> {
    use std::io::Read;
    let mut head = [0u8; 64];
    let mut z = flate2::read::ZlibDecoder::new(data.get(8..)?);
    let mut got = 0;
    while got < head.len() {
        match z.read(&mut head[got..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n,
        }
    }
    (got > 0).then(|| swf_rect(&head))
}

fn psd(c: &mut Cursor) -> Option<Info> {
    c.skip(11).then_some(())?;
    let d = c.read(8)?;
    Some(Info { height: be32(&d[0..4]), width: be32(&d[4..8]), ..Info::default() })
}

fn bmp(c: &mut Cursor) -> Option<Info> {
    c.skip(11).then_some(())?;
    let d = c.read(16)?;
    let size = le32(&d[0..4]);
    if size == 12 {
        Some(Info {
            width: (u32::from(d[5]) << 8) + u32::from(d[4]),
            height: (u32::from(d[7]) << 8) + u32::from(d[6]),
            bits: u32::from(d[11]),
            ..Info::default()
        })
    } else if size > 12 && (size <= 64 || size == 108 || size == 124) {
        Some(Info {
            width: le32(&d[4..8]),
            height: (le32(&d[8..12]) as i32).unsigned_abs(),
            bits: (u32::from(d[15]) << 8) + u32::from(d[14]),
            ..Info::default()
        })
    } else {
        None
    }
}

fn png(c: &mut Cursor) -> Option<Info> {
    c.skip(8).then_some(())?;
    let d = c.read(9)?;
    Some(Info { width: be32(&d[0..4]), height: be32(&d[4..8]), bits: u32::from(d[8]), ..Info::default() })
}

const M_SOS: u32 = 0xda;
const M_EOI: u32 = 0xd9;

/// php_next_marker: past the 0xFF bytes to a marker code (warning about
/// anything else in the way).
fn next_marker(ctx: &mut dyn Ctx, func: &str, c: &mut Cursor, ff_read: bool) -> u32 {
    if !ff_read {
        let mut extraneous = 0usize;
        loop {
            match c.getc() {
                None => return M_EOI,
                Some(0xff) => break,
                Some(_) => extraneous += 1,
            }
        }
        if extraneous > 0 {
            ctx.warn(&format!("{func}(): Corrupt JPEG data: {extraneous} extraneous bytes before marker"));
        }
    }
    loop {
        match c.getc() {
            None => return M_EOI,
            Some(0xff) => {}
            Some(m) => return u32::from(m),
        }
    }
}

/// php_skip_variable: over a block whose length comes first.
fn skip_variable(c: &mut Cursor) -> bool {
    let length = c.read2();
    if length < 2 {
        return false;
    }
    c.skip(i64::from(length - 2));
    true
}

fn jpeg(ctx: &mut dyn Ctx, func: &str, c: &mut Cursor, mut info: Option<&mut PhpArray>) -> Option<Info> {
    let mut result: Option<Info> = None;
    let mut ff_read = true;
    loop {
        let marker = next_marker(ctx, func, c, ff_read);
        ff_read = false;
        match marker {
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                if result.is_none() {
                    let length = c.read2();
                    let byte = |c: &mut Cursor| c.getc().map_or(u32::MAX, u32::from);
                    let bits = byte(c);
                    let height = c.read2();
                    let width = c.read2();
                    let channels = byte(c);
                    result = Some(Info { width, height, bits, channels });
                    if info.is_none() || length < 8 || !c.skip(i64::from(length) - 8) {
                        return result;
                    }
                } else if !skip_variable(c) {
                    return result;
                }
            }
            0xe0..=0xef => match info.as_deref_mut() {
                Some(app) => {
                    let length = c.read2() as usize;
                    if length < 2 {
                        return result;
                    }
                    let Some(body) = c.read(length - 2) else { return result };
                    let key = Key::from_string(PhpStr::from(format!("APP{}", marker - 0xe0).as_str()));
                    if app.get(&key).is_none() {
                        app.insert(key, Value::str(PhpStr::from(body)));
                    }
                }
                None => {
                    if !skip_variable(c) {
                        return result;
                    }
                }
            },
            M_SOS | M_EOI => return result,
            _ => {
                if !skip_variable(c) {
                    return result;
                }
            }
        }
    }
}

/// php_handle_tiff: the width and height tags of the first directory.
fn tiff(c: &mut Cursor, motorola: bool) -> Option<Info> {
    let get16 = |b: &[u8]| if motorola { (u32::from(b[0]) << 8) | u32::from(b[1]) } else { (u32::from(b[1]) << 8) | u32::from(b[0]) };
    let get32 = |b: &[u8]| if motorola { be32(b) } else { le32(b) };
    let ifd_addr = get32(c.read(4)?);
    c.skip(i64::from(ifd_addr) - 8).then_some(())?;
    let entries = get16(c.read(2)?) as usize;
    let dir = c.read(12 * entries + 4)?;
    let (mut width, mut height) = (0u64, 0u64);
    for i in 0..entries {
        let e = &dir[i * 12..i * 12 + 12];
        let value: u64 = match get16(&e[2..4]) {
            1 | 6 => u64::from(e[8]),
            3 => u64::from(get16(&e[8..10])),
            8 => i64::from(get16(&e[8..10]) as u16 as i16) as u64,
            4 => u64::from(get32(&e[8..12])),
            9 => i64::from(get32(&e[8..12]) as i32) as u64,
            _ => continue,
        };
        match get16(&e[0..2]) {
            0x0100 | 0xa002 => width = value,
            0x0101 | 0xa003 => height = value,
            _ => {}
        }
    }
    (width != 0 && height != 0).then(|| Info { width: width as u32, height: height as u32, ..Info::default() })
}

/// php_get_wbmp: type 0, then the multi-byte width and height (each at most 2048).
fn wbmp(data: &[u8]) -> Option<Info> {
    let mut c = Cursor { data, pos: 0 };
    if c.getc()? != 0 {
        return None;
    }
    while c.getc()? & 0x80 != 0 {}
    let mut number = || -> Option<u32> {
        let mut n = 0u32;
        loop {
            let i = c.getc()?;
            n = (n << 7) | u32::from(i & 0x7f);
            if n > 2048 {
                return None;
            }
            if i & 0x80 == 0 {
                return Some(n);
            }
        }
    };
    let width = number()?;
    let height = number()?;
    (width != 0 && height != 0).then_some(Info { width, height, ..Info::default() })
}

/// php_get_xbm: `#define name_width N` and `#define name_height N` lines.
fn xbm(data: &[u8]) -> Option<Info> {
    let (mut width, mut height) = (0u32, 0u32);
    for line in data.split_inclusive(|&b| b == b'\n') {
        let text = String::from_utf8_lossy(line);
        let mut words = text.split_whitespace();
        if words.next() != Some("#define") {
            continue;
        }
        let (Some(name), Some(value)) = (words.next(), words.next()) else { continue };
        let digits: String = value.chars().enumerate().take_while(|(i, ch)| ch.is_ascii_digit() || (*i == 0 && (*ch == '-' || *ch == '+'))).map(|(_, ch)| ch).collect();
        let Ok(value) = digits.parse::<i32>() else { continue };
        let kind = name.rsplit('_').next().unwrap_or(name);
        if kind == "width" {
            width = value as u32;
            if height != 0 {
                break;
            }
        }
        if kind == "height" {
            height = value as u32;
            if width != 0 {
                break;
            }
        }
    }
    (width != 0 && height != 0).then_some(Info { width, height, ..Info::default() })
}

/// php_handle_ico: the entry with the most bits per pixel (0 is 256).
fn ico(c: &mut Cursor) -> Option<Info> {
    let n = c.read(2)?;
    let count = (u32::from(n[1]) << 8) + u32::from(n[0]);
    if !(1..=255).contains(&count) {
        return None;
    }
    let mut r = Info::default();
    for _ in 0..count {
        let Some(d) = c.read(16) else { break };
        let bits = (u32::from(d[7]) << 8) + u32::from(d[6]);
        if bits >= r.bits {
            r.width = u32::from(d[0]);
            r.height = u32::from(d[1]);
            r.bits = bits;
        }
    }
    if r.width == 0 {
        r.width = 256;
    }
    if r.height == 0 {
        r.height = 256;
    }
    Some(r)
}

/// php_handle_webp: VP8 (lossy), VP8L (lossless) or VP8X (extended).
fn webp(c: &mut Cursor) -> Option<Info> {
    let b = c.read(18)?;
    if &b[0..3] != b"VP8" {
        return None;
    }
    let (w, h) = match b[3] {
        b' ' => (u32::from(b[14]) + ((u32::from(b[15]) & 0x3f) << 8), u32::from(b[16]) + ((u32::from(b[17]) & 0x3f) << 8)),
        b'L' => (
            u32::from(b[9]) + ((u32::from(b[10]) & 0x3f) << 8) + 1,
            (u32::from(b[10]) >> 6) + (u32::from(b[11]) << 2) + ((u32::from(b[12]) & 0xf) << 10) + 1,
        ),
        b'X' => (
            u32::from(b[12]) + (u32::from(b[13]) << 8) + (u32::from(b[14]) << 16) + 1,
            u32::from(b[15]) + (u32::from(b[16]) << 8) + (u32::from(b[17]) << 16) + 1,
        ),
        _ => return None,
    };
    Some(Info { width: w, height: h, bits: 8, ..Info::default() })
}

/// php_getimagesize_from_stream: [width, height, type, 'width="w" height="h"', bits?, channels?, mime].
fn size_of(ctx: &mut dyn Ctx, func: &str, data: &[u8], input: &str, info: Option<&mut PhpArray>) -> Value {
    let mut c = Cursor { data, pos: 0 };
    let kind = image_type(ctx, func, &mut c, input);
    let result = match kind {
        GIF => gif(&mut c),
        JPEG => jpeg(ctx, func, &mut c, info),
        PNG => png(&mut c),
        SWF => c.skip(5).then(|| c.read(32)).flatten().map(swf_rect),
        SWC => swc(data),
        PSD => psd(&mut c),
        BMP => bmp(&mut c),
        TIFF_II => tiff(&mut c, false),
        TIFF_MM => tiff(&mut c, true),
        WBMP => wbmp(data),
        XBM => xbm(data),
        ICO => ico(&mut c),
        WEBP => webp(&mut c),
        _ => None,
    };
    let Some(r) = result else { return Value::FALSE };
    let mut out = PhpArray::new();
    out.insert(Key::Int(0), Value::int(i64::from(r.width)));
    out.insert(Key::Int(1), Value::int(i64::from(r.height)));
    out.insert(Key::Int(2), Value::int(kind));
    out.insert(Key::Int(3), Value::str(format!("width=\"{}\" height=\"{}\"", r.width as i32, r.height as i32)));
    if r.bits != 0 {
        out.insert(Key::from_string(PhpStr::from("bits")), Value::int(i64::from(r.bits)));
    }
    if r.channels != 0 {
        out.insert(Key::from_string(PhpStr::from("channels")), Value::int(i64::from(r.channels)));
    }
    out.insert(Key::from_string(PhpStr::from("mime")), Value::str(mime(kind)));
    Value::array(out)
}

/// The by-reference $image_info, made an array (filled with a JPEG's APP markers).
fn with_info(ctx: &mut dyn Ctx, func: &str, args: &mut [Value], data: &[u8], input: &str) -> Value {
    if args.len() < 2 {
        return size_of(ctx, func, data, input, None);
    }
    let mut info = PhpArray::new();
    let r = size_of(ctx, func, data, input, Some(&mut info));
    if let Some(cell) = args.get(1).and_then(Value::as_reference) {
        *cell.borrow_mut() = Value::array(info);
    }
    r
}

/// getimagesize($filename, &$image_info = null).
fn getimagesize(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let path = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    if path.as_bytes().contains(&0) {
        return Err(ctx.throw("ValueError", "getimagesize(): Argument #1 ($filename) must not contain any null bytes".into()));
    }
    let shown = String::from_utf8_lossy(path.as_bytes()).into_owned();
    let p = shown.strip_prefix("file://").unwrap_or(&shown).to_string();
    let data = match std::fs::read(&p) {
        Ok(d) => d,
        Err(e) => {
            if args.len() > 1
                && let Some(cell) = args.get(1).and_then(Value::as_reference)
            {
                *cell.borrow_mut() = Value::array(PhpArray::new());
            }
            ctx.warn(&format!("getimagesize({shown}): Failed to open stream: {}", crate::file::os_error(&e)));
            return Ok(Value::FALSE);
        }
    };
    Ok(with_info(ctx, "getimagesize", args, &data, &shown))
}

/// getimagesizefromstring($string, &$image_info = null).
fn getimagesizefromstring(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let data = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    let shown = String::from_utf8_lossy(data.as_bytes()).into_owned();
    Ok(with_info(ctx, "getimagesizefromstring", args, data.as_bytes(), &shown))
}

fn mime(kind: i64) -> &'static str {
    match kind {
        GIF => "image/gif",
        JPEG => "image/jpeg",
        PNG => "image/png",
        SWF | SWC => "application/x-shockwave-flash",
        PSD => "image/psd",
        BMP => "image/bmp",
        TIFF_II | TIFF_MM => "image/tiff",
        IFF => "image/iff",
        WBMP => "image/vnd.wap.wbmp",
        JP2 => "image/jp2",
        XBM => "image/xbm",
        ICO => "image/vnd.microsoft.icon",
        WEBP => "image/webp",
        AVIF => "image/avif",
        _ => "application/octet-stream",
    }
}

fn image_type_to_mime_type(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::str(mime(arg(args, 0).to_int())))
}

/// image_type_to_extension($image_type, $include_dot = true).
fn image_type_to_extension(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let ext = match arg(args, 0).to_int() {
        GIF => "gif",
        JPEG => "jpeg",
        PNG => "png",
        SWF | SWC => "swf",
        PSD => "psd",
        BMP | WBMP => "bmp",
        TIFF_II | TIFF_MM => "tiff",
        IFF => "iff",
        JPC => "jpc",
        JP2 => "jp2",
        JPX => "jpx",
        JB2 => "jb2",
        XBM => "xbm",
        ICO => "ico",
        WEBP => "webp",
        AVIF => "avif",
        _ => return Ok(Value::FALSE),
    };
    let dot = args.len() < 2 || arg(args, 1).to_bool();
    Ok(Value::str(if dot { format!(".{ext}") } else { ext.to_string() }))
}
