//! The exif extension's reading side: exif_read_data(), exif_thumbnail(),
//! exif_tagname(), exif_imagetype(), for JPEG and TIFF files. Maker notes
//! are not parsed (the MakerNote tag is kept as its raw bytes).
//!
//! Reference: $PHP_SRC_PATH/ext/exif/exif.c

use crate::exif_tags::{GPS, IFD, IOP};
use crate::{R, arg, given, int, lossy, os_error};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "exif_read_data", run: exif_read_data, by_ref: &[] },
    Builtin { name: "exif_tagname", run: exif_tagname, by_ref: &[] },
    Builtin { name: "exif_imagetype", run: exif_imagetype, by_ref: &[] },
    Builtin { name: "exif_thumbnail", run: exif_thumbnail, by_ref: &[1, 2, 3] },
];

const SECTION_NAMES: [&str; 14] =
    ["FILE", "COMPUTED", "ANY_TAG", "IFD0", "THUMBNAIL", "COMMENT", "APP0", "EXIF", "FPIX", "GPS", "INTEROP", "APP12", "WINXP", "MAKERNOTE"];
const FILE: usize = 0;
const COMPUTED: usize = 1;
const ANY_TAG: usize = 2;
const IFD0: usize = 3;
const THUMBNAIL: usize = 4;
const COMMENT: usize = 5;
const EXIF: usize = 7;
const GPS_S: usize = 9;
const INTEROP: usize = 10;
const APP12: usize = 11;

const BYTES: [usize; 14] = [0, 1, 1, 2, 4, 8, 1, 1, 2, 4, 8, 4, 8, 4];
const F_BYTE: u16 = 1;
const F_STRING: u16 = 2;
const F_USHORT: u16 = 3;
const F_ULONG: u16 = 4;
const F_URATIONAL: u16 = 5;
const F_SBYTE: u16 = 6;
const F_UNDEFINED: u16 = 7;
const F_SSHORT: u16 = 8;
const F_SLONG: u16 = 9;
const F_SRATIONAL: u16 = 10;
const F_SINGLE: u16 = 11;
const F_DOUBLE: u16 = 12;

#[derive(Clone)]
enum Num {
    U(u32),
    I(i32),
    L(i64),
    UR(u32, u32),
    SR(i32, i32),
    F(f64),
}

#[derive(Clone)]
enum Val {
    Bytes(Vec<u8>),
    Nums(Vec<Num>),
    Null,
}

struct Item {
    name: String,
    val: Val,
}

#[derive(Default)]
struct Info {
    sections: Vec<Vec<Item>>,
    found: u32,
    mi: Option<bool>,
    file_type: i64,
    width: i64,
    height: i64,
    is_color: i64,
    exif_image_width: i64,
    focalplane_x_res: f64,
    focalplane_units: f64,
    aperture: f32,
    exposure: f32,
    distance: f32,
    user_comment: Option<Vec<u8>>,
    user_comment_encoding: Option<Vec<u8>>,
    copyright: Option<Vec<u8>>,
    copyright_photographer: Option<Vec<u8>>,
    copyright_editor: Option<Vec<u8>>,
    thumb_offset: usize,
    thumb_size: usize,
    thumb_filetype: i64,
    thumb_width: i64,
    thumb_height: i64,
    /// The TIFF block offsets are relative to (a JPEG's APP1 payload, or the file).
    tiff: Rc<Vec<u8>>,
    /// Where that block starts in the file.
    tiff_base: usize,
    nesting: u32,
}

impl Info {
    fn mi(&self) -> bool {
        self.mi.unwrap_or(false)
    }
    fn u16(&self, d: &[u8]) -> u32 {
        if d.len() < 2 { 0 } else if self.mi() { u32::from(u16::from_be_bytes([d[0], d[1]])) } else { u32::from(u16::from_le_bytes([d[0], d[1]])) }
    }
    fn u32(&self, d: &[u8]) -> u32 {
        if d.len() < 4 { 0 } else if self.mi() { u32::from_be_bytes([d[0], d[1], d[2], d[3]]) } else { u32::from_le_bytes([d[0], d[1], d[2], d[3]]) }
    }

    fn add(&mut self, section: usize, name: &str, val: Val) {
        while self.sections.len() <= section {
            self.sections.push(Vec::new());
        }
        self.sections[section].push(Item { name: name.to_string(), val });
        self.found |= 1 << section;
    }
    fn add_str(&mut self, section: usize, name: &str, v: &[u8]) {
        // A C string: it ends at its first NUL.
        let v = &v[..v.iter().position(|&b| b == 0).unwrap_or(v.len())];
        self.add(section, name, if v.is_empty() { Val::Null } else { Val::Bytes(v.to_vec()) });
    }
    fn add_int(&mut self, section: usize, name: &str, v: i64) {
        self.add(section, name, Val::Nums(vec![Num::L(v)]));
    }

    /// The numeric value at `v` as a float (exif_convert_any_format).
    fn num(&self, v: &[u8], format: u16) -> f64 {
        let rat = |signed: bool| {
            let (n, d) = (self.u32(v), self.u32(v.get(4..).unwrap_or(&[])));
            if d == 0 { 0.0 } else if signed { f64::from(n as i32) / f64::from(d as i32) } else { f64::from(n) / f64::from(d) }
        };
        match format {
            F_BYTE | F_UNDEFINED => f64::from(v.first().copied().unwrap_or(0)),
            F_SBYTE => f64::from(v.first().copied().unwrap_or(0) as i8),
            F_USHORT => f64::from(self.u16(v)),
            F_ULONG => f64::from(self.u32(v)),
            F_URATIONAL => rat(false),
            F_SSHORT => f64::from(self.u16(v) as u16 as i16),
            F_SLONG => f64::from(self.u32(v) as i32),
            F_SRATIONAL => rat(true),
            F_SINGLE => f64::from(f32::from_bits(self.u32(v))),
            F_DOUBLE => {
                let b: Vec<u8> = v.iter().take(8).copied().collect();
                if b.len() < 8 { 0.0 } else if self.mi() { f64::from_be_bytes(b.try_into().expect("8")) } else { f64::from_le_bytes(b.try_into().expect("8")) }
            }
            _ => 0.0,
        }
    }

    /// exif_convert_any_to_int, for an unsigned format.
    fn int(&self, v: &[u8], format: u16) -> i64 {
        match format {
            F_URATIONAL | F_SRATIONAL => {
                let (n, d) = (self.u32(v), self.u32(v.get(4..).unwrap_or(&[])));
                if d == 0 { 0 } else { i64::from(n / d) }
            }
            _ => self.num(v, format) as i64,
        }
    }

    fn add_tag(&mut self, section: usize, name: &str, tag: u16, format: u16, count: usize, value: &[u8]) {
        let val = match format {
            F_STRING => {
                let len = value.iter().take(count).position(|&b| b == 0).unwrap_or(count.min(value.len()));
                if len == 0 { Val::Null } else { Val::Bytes(value[..len].to_vec()) }
            }
            F_USHORT | F_ULONG | F_URATIONAL | F_SSHORT | F_SLONG | F_SRATIONAL | F_SINGLE | F_DOUBLE => {
                if count == 0 {
                    Val::Null
                } else {
                    let w = BYTES[usize::from(format)];
                    let mut nums = Vec::new();
                    for i in 0..count {
                        let Some(v) = value.get(i * w..(i + 1) * w) else { break };
                        nums.push(match format {
                            F_USHORT => Num::U(self.u16(v)),
                            F_ULONG => Num::U(self.u32(v)),
                            F_URATIONAL => Num::UR(self.u32(v), self.u32(&v[4..])),
                            F_SSHORT => Num::I(self.u16(v) as u16 as i16 as i32),
                            F_SLONG => Num::I(self.u32(v) as i32),
                            F_SRATIONAL => Num::SR(self.u32(v) as i32, self.u32(&v[4..]) as i32),
                            _ => Num::F(self.num(v, format)),
                        });
                    }
                    Val::Nums(nums)
                }
            }
            _ => {
                let mut n = count.min(value.len());
                if tag == 0x927C {
                    n = value[..n].iter().position(|&b| b == 0).unwrap_or(n);
                }
                if n == 0 { Val::Null } else { Val::Bytes(value[..n].to_vec()) }
            }
        };
        self.add(section, name, val);
    }
}

fn tag_name(table: &'static [(u16, &'static str)], tag: u16) -> Option<&'static str> {
    match tag {
        0xFFFF => Some("No tag value"),
        0xFFFE => Some("Computed value"),
        _ => table.iter().find(|(t, _)| *t == tag).map(|(_, n)| *n),
    }
}

fn table_of(section: usize) -> &'static [(u16, &'static str)] {
    match section {
        GPS_S => GPS,
        INTEROP => IOP,
        _ => IFD,
    }
}

/// exif_process_IFD_in_JPEG (also used for TIFF): the IFD at `off` in `info.tiff`.
fn process_ifd(info: &mut Info, off: usize, section: usize, parent_tag: i32) -> bool {
    if info.nesting > 8 {
        return false;
    }
    info.nesting += 1;
    let r = process_ifd_inner(info, off, section, parent_tag);
    info.nesting -= 1;
    r
}

fn process_ifd_inner(info: &mut Info, off: usize, section: usize, parent_tag: i32) -> bool {
    info.found |= 1 << IFD0;
    let tiff = Rc::clone(&info.tiff);
    let Some(head) = tiff.get(off..off + 2) else { return false };
    let n = info.u16(head) as usize;
    if tiff.get(off + 2..off + 2 + n * 12).is_none() {
        return false;
    }
    for de in 0..n {
        let entry = &tiff[off + 2 + 12 * de..off + 2 + 12 * de + 12];
        if !process_tag(info, entry, section) {
            return false;
        }
    }
    if section == THUMBNAIL {
        return true;
    }
    let Some(next) = tiff.get(off + 2 + n * 12..off + 2 + n * 12 + 4) else { return true };
    // 0x8769 (Exif) and 0x8825 (GPS) pointers end the chain.
    let next_off = if parent_tag != 0x8769 && parent_tag != 0x8825 { info.u32(next) as usize } else { 0 };
    if next_off != 0 {
        if next_off >= tiff.len() {
            return false;
        }
        return process_ifd(info, next_off, THUMBNAIL, 0);
    }
    true
}

fn process_tag(info: &mut Info, entry: &[u8], section: usize) -> bool {
    let tag = info.u16(entry) as u16;
    let mut format = info.u16(&entry[2..]) as u16;
    let components = info.u32(&entry[4..]) as usize;
    if format == 0 || format > 13 {
        format = F_BYTE;
    }
    let Some(byte_count) = components.checked_mul(BYTES[usize::from(format)]).filter(|&b| b <= i32::MAX as usize) else { return false };
    let tiff = Rc::clone(&info.tiff);
    let value: Vec<u8> = if byte_count > 4 {
        let offset = info.u32(&entry[8..]) as usize;
        match offset.checked_add(byte_count).and_then(|end| tiff.get(offset..end)) {
            Some(v) => v.to_vec(),
            None => return false,
        }
    } else {
        entry[8..8 + byte_count].to_vec()
    };
    info.found |= 1 << ANY_TAG;
    if section == THUMBNAIL {
        {
            let unsigned = match format {
                F_SSHORT => F_USHORT,
                F_SLONG => F_ULONG,
                F_SRATIONAL => F_URATIONAL,
                f => f,
            };
            if byte_count > 0 {
                match tag {
                    0x0100 | 0xA002 => info.thumb_width = info.int(&value, unsigned),
                    0x0101 | 0xA003 => info.thumb_height = info.int(&value, unsigned),
                    0x0111 | 0x0201 => info.thumb_offset = info.int(&value, unsigned) as usize,
                    0x0117 => {
                        info.thumb_filetype = if info.file_type == 7 || info.file_type == 8 { info.file_type } else { 8 };
                        info.thumb_size = info.int(&value, unsigned) as usize;
                    }
                    0x0202 => {
                        if info.thumb_filetype == 0 {
                            info.thumb_filetype = 2;
                            info.thumb_size = info.int(&value, unsigned) as usize;
                        }
                    }
                    _ => {}
                }
            }
        }
    } else if section == IFD0 || section == EXIF {
        let unsigned = match format {
            F_SSHORT => F_USHORT,
            F_SLONG => F_ULONG,
            F_SRATIONAL => F_URATIONAL,
            f => f,
        };
        match tag {
            0x8298 => {
                if byte_count > 1 {
                    let len = value.iter().position(|&b| b == 0).unwrap_or(byte_count);
                    if len > 0 {
                        if len < byte_count - 1 {
                            let photographer = value[..len].to_vec();
                            let editor = value[len + 1..byte_count].to_vec();
                            let mut both = photographer.clone();
                            both.extend_from_slice(b", ");
                            both.extend_from_slice(&editor);
                            info.copyright_photographer = Some(photographer);
                            info.copyright_editor = Some(editor);
                            info.copyright = Some(both);
                        } else {
                            info.copyright = Some(value[..byte_count].to_vec());
                        }
                    }
                }
            }
            0x9286 => {
                let (text, enc) = user_comment(info, &value);
                info.user_comment = Some(text);
                info.user_comment_encoding = enc;
            }
            0x829D => {
                if byte_count > 0 {
                    info.aperture = info.num(&value, format) as f32;
                }
            }
            0x9202 | 0x9205 => {
                if info.aperture == 0.0 && byte_count > 0 {
                    info.aperture = (info.num(&value, format) as f32 * std::f32::consts::LN_2 * 0.5).exp();
                }
            }
            0x9201 => {
                if info.exposure == 0.0 && byte_count > 0 {
                    info.exposure = (-(info.num(&value, format) as f32) * std::f32::consts::LN_2).exp();
                }
            }
            0x829A => info.exposure = -1.0,
            0xA002 => {
                if byte_count > 0 {
                    info.exif_image_width = info.int(&value, unsigned);
                }
            }
            0xA20E => {
                if byte_count > 0 {
                    info.focalplane_x_res = info.num(&value, format);
                }
            }
            0x9206 => {
                if byte_count > 0 {
                    info.distance = info.num(&value, format) as f32;
                }
            }
            0xA210 => {
                if byte_count > 0 {
                    match info.int(&value, format) {
                        1 | 2 => info.focalplane_units = 25.4,
                        3 => info.focalplane_units = 10.0,
                        4 => info.focalplane_units = 1.0,
                        5 => info.focalplane_units = 0.001,
                        _ => {}
                    }
                }
            }
            0x8769 | 0x8825 | 0xA005 => {
                if byte_count > 0 {
                    let sub = match tag {
                        0x8769 => EXIF,
                        0x8825 => GPS_S,
                        _ => INTEROP,
                    };
                    info.found |= 1 << sub;
                    let off = info.u32(&value) as usize;
                    if tiff.get(off..).is_none() {
                        return false;
                    }
                    if !process_ifd(info, off, sub, i32::from(tag)) {
                        return false;
                    }
                }
            }
            _ => {}
        }
    }
    let name = match tag_name(table_of(section), tag) {
        Some(n) => n.to_string(),
        None => format!("UndefinedTag:0x{tag:04X}"),
    };
    info.add_tag(section, &name, tag, format, components, &value);
    true
}

/// exif_process_user_comment: (text, encoding).
fn user_comment(info: &Info, v: &[u8]) -> (Vec<u8>, Option<Vec<u8>>) {
    let mut enc = None;
    let mut v = v;
    if v.len() >= 8 {
        let head = &v[..8];
        if head == b"UNICODE\0" {
            enc = Some(b"UNICODE".to_vec());
            let mut body = &v[8..];
            let mut be = info.mi();
            if body.starts_with(&[0xFE, 0xFF]) {
                be = true;
                body = &body[2..];
            } else if body.starts_with(&[0xFF, 0xFE]) {
                be = false;
                body = &body[2..];
            }
            let units: Vec<u16> = body.chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
            return (String::from_utf16_lossy(&units).into_bytes(), enc);
        } else if head == b"ASCII\0\0\0" {
            enc = Some(b"ASCII".to_vec());
            v = &v[8..];
        } else if head == b"JIS\0\0\0\0\0" {
            return (v[8..].to_vec(), Some(b"JIS".to_vec()));
        } else if head == [0u8; 8] {
            enc = Some(b"UNDEFINED".to_vec());
            v = &v[8..];
        }
    }
    let mut t = v.to_vec();
    while t.len() > 1 && t.last() == Some(&b' ') {
        t.pop();
    }
    let n = t.iter().position(|&b| b == 0).unwrap_or(t.len());
    t.truncate(n);
    (t, enc)
}

/// The JPEG's segments up to the scan: Exif APP1, comments, SOF sizes.
fn scan_jpeg(info: &mut Info, d: &[u8]) -> bool {
    let mut i = 2;
    loop {
        // A marker: 0xFF (fill bytes) and its code.
        let mut marker = 0;
        while i < d.len() {
            marker = d[i];
            i += 1;
            if marker != 0xff {
                break;
            }
        }
        if i > d.len() || marker == 0xff {
            return false;
        }
        let Some(len) = d.get(i..i + 2).map(|b| (usize::from(b[0]) << 8) | usize::from(b[1])) else { return false };
        if len < 2 {
            return false;
        }
        let Some(seg) = d.get(i..i + len) else { return false };
        i += len;
        match marker {
            0xDA => return true,
            0xD9 => return info.found & !(1 << COMPUTED) != 0,
            0xFE => info.add(COMMENT, "Comment", Val::Bytes(seg[2..].to_vec())),
            0xE1 => {
                if info.found & (1 << IFD0) == 0 {
                    app1(info, seg, i - len);
                }
            }
            0xEC => {
                let body = &seg[2..];
                let l1 = body.iter().position(|&b| b == 0).unwrap_or(body.len());
                if l1 > 0 {
                    info.add(APP12, "Company", Val::Bytes(body[..l1].to_vec()));
                    if body.len() > l1 + 1 {
                        let rest = &body[l1 + 1..];
                        let l2 = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
                        info.add(APP12, "Info", if l2 == 0 { Val::Null } else { Val::Bytes(rest[..l2].to_vec()) });
                    }
                }
            }
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                if seg.len() - 2 < 6 {
                    return false;
                }
                info.height = (i64::from(seg[3]) << 8) | i64::from(seg[4]);
                info.width = (i64::from(seg[5]) << 8) | i64::from(seg[6]);
                info.is_color = i64::from(seg[7] == 3);
            }
            _ => {}
        }
    }
}

fn app1(info: &mut Info, seg: &[u8], seg_at: usize) {
    if seg.len() <= 8 || &seg[2..8] != b"Exif\0\0" {
        return;
    }
    let tiff = &seg[8..];
    if tiff.len() < 2 {
        return;
    }
    info.mi = match &tiff[..2] {
        b"II" => Some(false),
        b"MM" => Some(true),
        _ => return,
    };
    if tiff.len() < 8 {
        return;
    }
    let (magic, ifd) = (info.u16(&tiff[2..]), info.u32(&tiff[4..]) as usize);
    if magic != 0x2a || ifd < 8 || ifd > tiff.len() {
        return;
    }
    info.found |= 1 << IFD0;
    info.tiff = Rc::new(tiff.to_vec());
    info.tiff_base = seg_at + 8;
    process_ifd(info, ifd, IFD0, 0);
}

fn mime(t: i64) -> &'static str {
    match t {
        1 => "image/gif",
        2 => "image/jpeg",
        3 => "image/png",
        4 | 13 => "application/x-shockwave-flash",
        5 => "image/psd",
        6 => "image/bmp",
        7 | 8 => "image/tiff",
        9 => "application/octet-stream",
        10 => "image/jp2",
        14 => "image/iff",
        15 => "image/vnd.wap.wbmp",
        16 => "image/xbm",
        17 => "image/vnd.microsoft.icon",
        18 => "image/webp",
        19 => "image/avif",
        _ => "application/octet-stream",
    }
}

/// php_getimagetype on the first bytes.
fn image_type(d: &[u8]) -> i64 {
    if d.starts_with(b"GIF") {
        1
    } else if d.starts_with(&[0xff, 0xd8, 0xff]) {
        2
    } else if d.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        3
    } else if d.starts_with(b"FWS") {
        4
    } else if d.starts_with(b"CWS") {
        13
    } else if d.starts_with(b"8BPS") {
        5
    } else if d.starts_with(b"BM") {
        6
    } else if d.starts_with(b"II\x2a\0") {
        7
    } else if d.starts_with(b"MM\0\x2a") {
        8
    } else if d.starts_with(&[0xff, 0x4f, 0xff]) {
        9
    } else if d.len() >= 12 && d[4..8] == *b"jP  " {
        10
    } else if d.starts_with(b"FORM") {
        14
    } else if d.starts_with(&[0, 0, 1, 0]) {
        17
    } else if d.len() >= 12 && &d[..4] == b"RIFF" && &d[8..12] == b"WEBP" {
        18
    } else if d.len() >= 12 && &d[4..8] == b"ftyp" && (&d[8..12] == b"avif" || &d[8..12] == b"avis") {
        19
    } else {
        0
    }
}

struct Read {
    info: Info,
    file_name: Option<String>,
    file_time: i64,
    file_size: usize,
    data: Vec<u8>,
}

fn read_file(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Option<Read>, p8_runtime::ext::Throw> {
    let a0 = arg(args, 0);
    let (data, name, time): (Vec<u8>, Option<String>, i64) = if let Some(r) = p8_runtime::stream::resource_of(&a0) {
        let Some(res) = p8_runtime::stream::resource(&r) else { return Ok(None) };
        let mut s = res.stream.borrow_mut();
        let at = s.tell();
        s.seek(0, 0);
        let data = s.read_all();
        s.seek(at, 0);
        (data, None, 0)
    } else {
        let path = a0.to_php_string().unwrap_or_else(PhpStr::empty);
        if path.as_bytes().is_empty() {
            return Err(ctx.throw("ValueError", "exif_read_data(): Argument #1 ($file) must not be empty".into()));
        }
        if path.as_bytes().contains(&0) {
            return Err(ctx.throw("ValueError", "exif_read_data(): Argument #1 ($file) must not contain any null bytes".into()));
        }
        let shown = lossy(path.as_bytes());
        let p = shown.strip_prefix("file://").unwrap_or(&shown).to_string();
        let (data, time) = if shown.contains("://") && !shown.starts_with("file://") || shown.starts_with("data:") {
            match ctx.call(&Value::str("file_get_contents"), &mut [Value::str(shown.as_str())])? {
                Value::Str(s) => (s.as_bytes().to_vec(), 0),
                _ => {
                    ctx.warn("exif_read_data(): Unable to open file");
                    return Ok(None);
                }
            }
        } else {
            match std::fs::metadata(&p) {
                Ok(m) if !m.is_file() => {
                    ctx.warn(&format!("exif_read_data({}): Not a file", basename(&p)));
                    return Ok(None);
                }
                Ok(m) => {
                    let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
                    match std::fs::read(&p) {
                        Ok(d) => (d, t),
                        Err(e) => {
                            let _ = e;
                            ctx.warn("exif_read_data(): Unable to open file");
                            return Ok(None);
                        }
                    }
                }
                Err(_) => {
                    ctx.warn("exif_read_data(): Unable to open file");
                    return Ok(None);
                }
            }
        };
        (data, Some(basename(&p)), time)
    };
    let mut info = Info { mi: None, focalplane_units: 0.0, ..Info::default() };
    let prefix = match &name {
        Some(n) => format!("exif_read_data({n})"),
        None => "exif_read_data()".to_string(),
    };
    let size = data.len();
    let ok = if size >= 2 {
        if data[0] == 0xff && data[1] == 0xd8 {
            info.file_type = 2;
            if scan_jpeg(&mut info, &data) {
                true
            } else {
                ctx.warn(&format!("{prefix}: Invalid JPEG file"));
                false
            }
        } else if size >= 8 {
            let tiff = if data.starts_with(b"II\x2a\0") {
                Some((7, false))
            } else if data.starts_with(b"MM\0\x2a") {
                Some((8, true))
            } else {
                None
            };
            match tiff {
                Some((ty, mi)) => {
                    info.file_type = ty;
                    info.mi = Some(mi);
                    info.found |= 1 << IFD0;
                    info.tiff = Rc::new(data.clone());
                    let off = info.u32(&data[4..]) as usize;
                    if process_ifd(&mut info, off, IFD0, 0) {
                        true
                    } else {
                        ctx.warn(&format!("{prefix}: Invalid TIFF file"));
                        false
                    }
                }
                None => {
                    ctx.warn(&format!("{prefix}: File not supported"));
                    false
                }
            }
        } else {
            false
        }
    } else {
        ctx.warn(&format!("{prefix}: File too small ({size})"));
        false
    };
    if !ok {
        return Ok(None);
    }
    info.file_type = if info.file_type == 0 { image_type(&data) } else { info.file_type };
    Ok(Some(Read { info, file_name: name, file_time: time, file_size: size, data }))
}

fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

fn bytes_val(b: &[u8]) -> Value {
    Value::Str(PhpStr::new(b.to_vec()))
}

fn key(name: &str) -> Key {
    // Numeric-looking names become integer keys, as PHP's add_assoc does.
    Key::from_value(&Value::str(name)).map(|(k, _)| k).unwrap_or_else(|_| Key::from_string(name.into()))
}

fn item_value(it: &Item) -> Value {
    match &it.val {
        Val::Null => Value::Null,
        Val::Bytes(b) => bytes_val(b),
        Val::Nums(n) => {
            let one = |x: &Num| match x {
                Num::U(u) => Value::Int(i64::from(*u as i32)),
                Num::I(i) => Value::Int(i64::from(*i)),
                Num::L(l) => Value::Int(*l),
                Num::UR(a, b) => Value::str(format!("{a}/{b}").as_str()),
                Num::SR(a, b) => Value::str(format!("{a}/{b}").as_str()),
                Num::F(f) => Value::Float(*f),
            };
            if n.len() == 1 { one(&n[0]) } else { Value::Array(Rc::new(PhpArray::from_list(n.iter().map(one).collect()))) }
        }
    }
}

/// add_assoc_image_info: a section's items into `out` (or a sub-array).
fn add_section(out: &mut PhpArray, sub_array: bool, info: &Info, section: usize) {
    let Some(items) = info.sections.get(section).filter(|i| !i.is_empty()) else { return };
    let mut target = if sub_array { PhpArray::new() } else { std::mem::take(out) };
    let mut idx = 0;
    for it in items {
        if section == COMMENT {
            let _ = target.push(item_value(it));
            idx += 1;
        } else {
            target.insert(key(&it.name), item_value(it));
        }
    }
    let _ = idx;
    if sub_array {
        out.insert(Key::from_string(SECTION_NAMES[section].into()), Value::Array(Rc::new(target)));
    } else {
        *out = target;
    }
}

fn sections_list(found: u32) -> String {
    (0..SECTION_NAMES.len()).filter(|&i| found & (1 << i) != 0).map(|i| SECTION_NAMES[i]).collect::<Vec<_>>().join(", ")
}

fn build(r: &mut Read, sub_arrays: bool, want_thumb: bool) -> PhpArray {
    let info = &mut r.info;
    let sections = sections_list(info.found);
    info.found |= (1 << COMPUTED) | 1;
    let file_name = r.file_name.clone().unwrap_or_default();
    if r.file_name.is_some() {
        info.add_str(FILE, "FileName", file_name.as_bytes());
    }
    info.add_int(FILE, "FileDateTime", r.file_time);
    info.add_int(FILE, "FileSize", r.file_size as i64);
    info.add_int(FILE, "FileType", info.file_type);
    info.add_str(FILE, "MimeType", mime(info.file_type).as_bytes());
    info.add(FILE, "SectionsFound", Val::Bytes(sections.as_bytes().to_vec()));
    if info.width > 0 && info.height > 0 {
        let html = format!("width=\"{}\" height=\"{}\"", info.width, info.height);
        info.add_str(COMPUTED, "html", html.as_bytes());
        info.add_int(COMPUTED, "Height", info.height);
        info.add_int(COMPUTED, "Width", info.width);
    }
    info.add_int(COMPUTED, "IsColor", info.is_color);
    if let Some(mi) = info.mi {
        info.add_int(COMPUTED, "ByteOrderMotorola", i64::from(mi));
    }
    if info.focalplane_x_res != 0.0 {
        let ccd = (info.exif_image_width as f64 * info.focalplane_units / info.focalplane_x_res) as f32;
        if ccd != 0.0 {
            info.add_str(COMPUTED, "CCDWidth", format!("{}mm", ccd as i64).as_bytes());
        }
    }
    if info.exposure > 0.0 {
        let recip = 0.5 + 1.0 / info.exposure;
        let text = if info.exposure <= 0.5 && recip < i32::MAX as f32 {
            format!("{:.3} s (1/{})", info.exposure, recip as i32)
        } else {
            format!("{:.3} s", info.exposure)
        };
        info.add_str(COMPUTED, "ExposureTime", text.as_bytes());
    }
    if info.aperture != 0.0 {
        info.add_str(COMPUTED, "ApertureFNumber", format!("f/{:.1}", info.aperture).as_bytes());
    }
    if info.distance != 0.0 {
        if info.distance < 0.0 {
            info.add_str(COMPUTED, "FocusDistance", b"Infinite");
        } else {
            info.add_str(COMPUTED, "FocusDistance", format!("{:.2}m", info.distance).as_bytes());
        }
    }
    if let Some(c) = info.user_comment.clone() {
        info.add(COMPUTED, "UserComment", if c.is_empty() { Val::Null } else { Val::Bytes(c) });
        if let Some(e) = info.user_comment_encoding.clone().filter(|e| !e.is_empty()) {
            info.add_str(COMPUTED, "UserCommentEncoding", &e);
        }
    }
    for (name, v) in [("Copyright", info.copyright.clone()), ("Copyright.Photographer", info.copyright_photographer.clone()), ("Copyright.Editor", info.copyright_editor.clone())] {
        if let Some(v) = v {
            info.add_str(COMPUTED, name, &v);
        }
    }
    if info.thumb_size > 0 {
        if want_thumb && let Some(t) = r.data.get(thumb_start(info)..thumb_start(info) + info.thumb_size) {
            let t = t.to_vec();
            info.add(THUMBNAIL, "THUMBNAIL", Val::Bytes(t));
        }
        if want_thumb && (info.thumb_width == 0 || info.thumb_height == 0) {
            if let Some(t) = thumb_bytes(&r.data, info) {
                if let Some((w, h)) = jpeg_size(t) {
                    info.thumb_width = w;
                    info.thumb_height = h;
                }
            }
        }
        info.add_int(COMPUTED, "Thumbnail.FileType", info.thumb_filetype);
        info.add_str(COMPUTED, "Thumbnail.MimeType", mime(info.thumb_filetype).as_bytes());
    }
    if info.thumb_width > 0 && info.thumb_height > 0 {
        info.add_int(COMPUTED, "Thumbnail.Height", info.thumb_height);
        info.add_int(COMPUTED, "Thumbnail.Width", info.thumb_width);
    }
    let mut out = PhpArray::new();
    let info = &r.info;
    add_section(&mut out, sub_arrays, info, FILE);
    add_section(&mut out, true, info, COMPUTED);
    add_section(&mut out, sub_arrays, info, ANY_TAG);
    add_section(&mut out, sub_arrays, info, IFD0);
    add_section(&mut out, true, info, THUMBNAIL);
    add_section(&mut out, true, info, COMMENT);
    for s in [EXIF, 8, GPS_S, INTEROP, APP12, 12, 13] {
        add_section(&mut out, sub_arrays, info, s);
    }
    out
}

/// Where the thumbnail starts in the file's bytes (a JPEG's offsets are into the APP1 TIFF block).
fn thumb_start(info: &Info) -> usize {
    info.thumb_offset + if info.file_type == 2 { info.tiff_base } else { 0 }
}

fn thumb_bytes<'a>(data: &'a [u8], info: &Info) -> Option<&'a [u8]> {
    let start = thumb_start(info);
    data.get(start..start.checked_add(info.thumb_size)?)
}

fn jpeg_size(t: &[u8]) -> Option<(i64, i64)> {
    if !t.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut i = 2;
    while i + 4 <= t.len() {
        if t[i] != 0xff {
            i += 1;
            continue;
        }
        let m = t[i + 1];
        if m == 0xff {
            i += 1;
            continue;
        }
        let len = (usize::from(t[i + 2]) << 8) | usize::from(t[i + 3]);
        if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) && i + 9 <= t.len() {
            return Some(((i64::from(t[i + 7]) << 8) | i64::from(t[i + 8]), (i64::from(t[i + 5]) << 8) | i64::from(t[i + 6])));
        }
        if m == 0xda || len < 2 {
            return None;
        }
        i += 2 + len;
    }
    None
}

fn exif_read_data(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let needed: u32 = if given(args, 1) {
        let text = format!(",{},", lossy(arg(args, 1).to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).replace(' ', ","));
        (0..SECTION_NAMES.len()).filter(|&i| text.contains(&format!(",{},", SECTION_NAMES[i]))).fold(0, |m, i| m | (1 << i))
    } else {
        0
    };
    let sub = arg(args, 2).to_bool();
    let thumb = arg(args, 3).to_bool();
    let Some(mut r) = read_file(ctx, args)? else { return Ok(Value::Bool(false)) };
    if needed != 0 && needed & (r.info.found | (1 << COMPUTED) | 1) == 0 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Array(Rc::new(build(&mut r, sub, thumb))))
}

fn exif_tagname(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let tag = int(args, 0);
    if tag < 0 {
        return Ok(Value::Bool(false));
    }
    Ok(match u16::try_from(tag).ok().and_then(|t| tag_name(IFD, t)) {
        Some(n) => Value::str(n),
        None => Value::Bool(false),
    })
}

fn exif_imagetype(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let path = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    if path.as_bytes().contains(&0) {
        return Err(ctx.throw("ValueError", "exif_imagetype(): Argument #1 ($filename) must not contain any null bytes".into()));
    }
    let shown = lossy(path.as_bytes());
    let p = shown.strip_prefix("file://").unwrap_or(&shown);
    let data = if shown.contains("://") && !shown.starts_with("file://") {
        match ctx.call(&Value::str("file_get_contents"), &mut [Value::str(shown.as_str())])? {
            Value::Str(s) => s.as_bytes().to_vec(),
            _ => return Ok(Value::Bool(false)),
        }
    } else {
        match std::fs::read(p) {
            Ok(d) => d,
            Err(e) => {
                ctx.warn(&format!("exif_imagetype({shown}): Failed to open stream: {}", os_error(&e)));
                return Ok(Value::Bool(false));
            }
        }
    };
    Ok(match image_type(&data) {
        0 => {
            if data.len() < 12 {
                ctx.notice(&format!("exif_imagetype({shown}): Error reading from {shown}!"));
            }
            Value::Bool(false)
        }
        t => Value::Int(t),
    })
}

fn set_ref(args: &[Value], i: usize, v: Value) {
    if let Some(Value::Ref(cell)) = args.get(i) {
        *cell.borrow_mut() = v;
    }
}

fn exif_thumbnail(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some(mut r) = read_file(ctx, args)? else { return Ok(Value::Bool(false)) };
    let _ = build(&mut r, false, true);
    let info = &r.info;
    let Some(t) = thumb_bytes(&r.data, info).filter(|t| !t.is_empty()) else {
        ctx.notice("exif_thumbnail(): No thumbnail data");
        return Ok(Value::Bool(false));
    };
    if args.len() > 3 {
        set_ref(args, 3, Value::Int(info.thumb_filetype));
    }
    if args.len() > 2 {
        set_ref(args, 1, Value::Int(info.thumb_width));
        set_ref(args, 2, Value::Int(info.thumb_height));
    }
    Ok(bytes_val(t))
}
