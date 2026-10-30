//! A zip archive in memory: reading the container, the entries and the
//! changes made to them, and writing it out again. What is observable
//! follows libzip (the library PHP's zip extension wraps): entries keep their
//! index until the archive is closed, a deleted entry still counts, a change
//! can be undone until close. Encoded bytes (the deflate stream) are not
//! libzip's.
//!
//! Reference: $PHP_SRC_PATH/ext/zip/php_zip.c; the zip format (APPNOTE.TXT)

use std::rc::Rc;

pub const ER_MULTIDISK: i32 = 1;
pub const ER_READ: i32 = 5;
pub const ER_CRC: i32 = 7;
pub const ER_NOENT: i32 = 9;
pub const ER_EXISTS: i32 = 10;
pub const ER_OPEN: i32 = 11;
pub const ER_CHANGED: i32 = 15;
pub const ER_COMPNOTSUPP: i32 = 16;
pub const ER_INVAL: i32 = 18;
pub const ER_NOZIP: i32 = 19;
pub const ER_INCONS: i32 = 21;
pub const ER_DELETED: i32 = 23;
pub const ER_ENCRNOTSUPP: i32 = 24;
pub const ER_RDONLY: i32 = 25;
pub const ER_NOPASSWD: i32 = 26;
pub const ER_COMPRESSED_DATA: i32 = 31;
pub const ER_TRUNCATED_ZIP: i32 = 35;

pub const CM_DEFAULT: i32 = -1;
pub const CM_STORE: i32 = 0;
pub const CM_DEFLATE: i32 = 8;

/// libzip's error: a code, and the system errno for the codes that carry one.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Error {
    pub code: i32,
    pub sys: i32,
}

impl Error {
    pub fn new(code: i32) -> Error {
        Error { code, sys: 0 }
    }

    pub fn io(code: i32, e: &std::io::Error) -> Error {
        Error { code, sys: e.raw_os_error().unwrap_or(0) }
    }

    /// zip_error_strerror
    pub fn message(&self) -> String {
        const MESSAGES: [&str; 36] = [
            "No error",
            "Multi-disk zip archives not supported",
            "Renaming temporary file failed",
            "Closing zip archive failed",
            "Seek error",
            "Read error",
            "Write error",
            "CRC error",
            "Containing zip archive was closed",
            "No such file",
            "File already exists",
            "Can't open file",
            "Failure to create temporary file",
            "Zlib error",
            "Malloc failure",
            "Entry has been changed",
            "Compression method not supported",
            "Premature end of file",
            "Invalid argument",
            "Not a zip archive",
            "Internal error",
            "Zip archive inconsistent",
            "Can't remove file",
            "Entry has been deleted",
            "Encryption method not supported",
            "Read-only archive",
            "No password provided",
            "Wrong password provided",
            "Operation not supported",
            "Resource still in use",
            "Tell error",
            "Compressed data invalid",
            "Operation cancelled",
            "Unexpected length of data",
            "Not allowed in torrentzip",
            "Possibly truncated or corrupted zip archive",
        ];
        let base = MESSAGES.get(self.code as usize).copied().unwrap_or("Unknown error");
        // Errors that carry a system errno (libzip's N/S/Z/L types).
        let with_sys = matches!(self.code, 2 | 3 | 4 | 5 | 6 | 11 | 12 | 22 | 30);
        if with_sys && self.sys != 0 { format!("{base}: {}", strerror(self.sys)) } else { base.to_string() }
    }
}

pub fn strerror(errno: i32) -> &'static str {
    match errno {
        1 => "Operation not permitted",
        2 => "No such file or directory",
        13 => "Permission denied",
        17 => "File exists",
        20 => "Not a directory",
        21 => "Is a directory",
        22 => "Invalid argument",
        28 => "No space left on device",
        30 => "Read-only file system",
        _ => "Unknown error",
    }
}

/// Where an entry's bytes come from.
#[derive(Clone)]
pub enum Source {
    /// The archive as read, and the offset of the entry's local header.
    Raw(Rc<Vec<u8>>, usize),
    /// Uncompressed bytes given by the script.
    Memory(Rc<Vec<u8>>),
    /// A file read when the archive is written (or when the entry is read).
    /// `loaded`: the bytes, read when the file was added (FL_OPEN_FILE_NOW).
    File { path: String, start: u64, len: i64, loaded: Option<Rc<Vec<u8>>> },
}

#[derive(Clone)]
pub struct Rec {
    pub name: Vec<u8>,
    pub comment: Vec<u8>,
    /// -1: choose when writing.
    pub method: i32,
    pub level: u32,
    pub crc: u32,
    pub size: u64,
    pub comp_size: u64,
    pub mtime: i64,
    pub opsys: u8,
    pub attr: u32,
    pub flags: u16,
    pub extra_local: Vec<u8>,
    pub extra_central: Vec<u8>,
    pub source: Source,
    /// The data in the source differs from what the original held.
    pub data_changed: bool,
    /// The compression method was set: the data is compressed again.
    pub recompress: bool,
    /// The method the bytes of a `Source::Raw` are compressed with.
    pub src_method: i32,
    pub encrypted: bool,
}

impl Rec {
    pub fn new(name: Vec<u8>, source: Source) -> Rec {
        let size = match &source {
            Source::Memory(d) => d.len() as u64,
            _ => 0,
        };
        Rec {
            name,
            comment: Vec::new(),
            method: CM_DEFAULT,
            level: 0,
            crc: 0,
            size,
            comp_size: 0,
            mtime: now(),
            opsys: 3,
            attr: 0,
            flags: 0,
            extra_local: Vec::new(),
            extra_central: Vec::new(),
            source,
            data_changed: true,
            recompress: false,
            src_method: CM_STORE,
            encrypted: false,
        }
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Clone)]
pub struct Slot {
    pub orig: Option<Rec>,
    pub cur: Rec,
    pub deleted: bool,
}

pub struct Stat {
    pub name: Vec<u8>,
    pub index: usize,
    pub crc: u32,
    pub size: u64,
    pub mtime: i64,
    pub comp_size: i64,
    pub comp_method: i32,
    pub encryption: i32,
}

#[derive(Default)]
pub struct Archive {
    pub slots: Vec<Slot>,
    pub comment: Vec<u8>,
    pub orig_comment: Vec<u8>,
    pub readonly: bool,
    pub password: Option<Vec<u8>>,
    /// The archive read is a torrentzip (ZipArchive::AFL_IS_TORRENTZIP).
    pub torrent: bool,
    /// Write it as one (AFL_WANT_TORRENTZIP).
    pub want_torrent: bool,
}

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn u64le(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

// ---- time ----

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn dos_to_unix(time: u16, date: u16) -> i64 {
    let year = 1980 + i64::from(date >> 9);
    let month = i64::from((date >> 5) & 15).clamp(1, 12);
    let day = i64::from(date & 31).max(1);
    let (h, mi, s) = (i64::from(time >> 11), i64::from((time >> 5) & 63), i64::from(time & 31) * 2);
    days_from_civil(year, month, day) * 86400 + h * 3600 + mi * 60 + s
}

pub fn unix_to_dos(t: i64) -> (u16, u16) {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    if y < 1980 {
        return (0, (1 << 5) | 1);
    }
    if y > 2107 {
        return ((23 << 11) | (59 << 5) | 29, (127 << 9) | (12 << 5) | 31);
    }
    let time = ((secs / 3600) << 11) | (((secs % 3600) / 60) << 5) | ((secs % 60) / 2);
    (time as u16, (((y - 1980) << 9) | (m << 5) | d) as u16)
}

// ---- reading ----

impl Archive {
    pub fn new() -> Archive {
        Archive::default()
    }

    /// Read an archive. `checkcons` is ZipArchive::CHECKCONS.
    pub fn parse(raw: Vec<u8>, checkcons: bool) -> Result<Archive, Error> {
        let raw = Rc::new(raw);
        let b = &raw[..];
        if b.len() < 22 {
            return Err(Error::new(ER_NOZIP));
        }
        let lowest = b.len().saturating_sub(22 + 65535);
        let mut eocd = None;
        let mut i = b.len() - 22;
        loop {
            if b[i..i + 4] == [0x50, 0x4b, 5, 6] {
                let clen = usize::from(u16le(b, i + 20));
                if i + 22 + clen <= b.len() && (!checkcons || i + 22 + clen == b.len()) {
                    eocd = Some(i);
                    break;
                }
                if i + 22 + clen <= b.len() {
                    eocd = Some(i);
                    break;
                }
            }
            if i == lowest {
                break;
            }
            i -= 1;
        }
        let Some(eocd) = eocd else { return Err(Error::new(ER_NOZIP)) };
        let (disk, cd_disk) = (u16le(b, eocd + 4), u16le(b, eocd + 6));
        let mut count = u64::from(u16le(b, eocd + 10));
        let mut cd_size = u64::from(u32le(b, eocd + 12));
        let mut cd_off = u64::from(u32le(b, eocd + 16));
        let clen = usize::from(u16le(b, eocd + 20));
        let comment = b[eocd + 22..(eocd + 22 + clen).min(b.len())].to_vec();
        if disk != 0 || cd_disk != 0 {
            return Err(Error::new(ER_MULTIDISK));
        }
        let mut cd_end = eocd as u64;
        // Zip64: the locator just before the end record.
        if eocd >= 20 && b[eocd - 20..eocd - 16] == [0x50, 0x4b, 6, 7] {
            let at = u64le(b, eocd - 12) as usize;
            if at + 56 <= b.len() && b[at..at + 4] == [0x50, 0x4b, 6, 6] {
                count = u64le(b, at + 32);
                cd_size = u64le(b, at + 40);
                cd_off = u64le(b, at + 48);
                cd_end = at as u64;
            }
        }
        // A central directory reaching past the end record is no archive.
        if cd_off.checked_add(cd_size).is_none_or(|end| end > cd_end) {
            return Err(Error::new(ER_INCONS));
        }
        // An archive with bytes in front of it (a self-extractor): offsets shift.
        let mut shift = 0i64;
        if cd_off.checked_add(cd_size) != Some(cd_end) {
            let want = cd_end as i64 - cd_size as i64;
            if want >= 0 && (want as usize) + 4 <= b.len() && b[want as usize..want as usize + 4] == [0x50, 0x4b, 1, 2] {
                shift = want - cd_off as i64;
            } else if checkcons {
                return Err(Error::new(ER_INCONS));
            }
        }
        let mut p = (cd_off as i64 + shift) as usize;
        if p > b.len() {
            return Err(Error::new(ER_NOZIP));
        }
        let mut slots = Vec::new();
        for _ in 0..count {
            if p + 46 > b.len() || b[p..p + 4] != [0x50, 0x4b, 1, 2] {
                return Err(Error::new(if checkcons { ER_INCONS } else { ER_NOZIP }));
            }
            let made = u16le(b, p + 4);
            let flags = u16le(b, p + 8);
            let method = i32::from(u16le(b, p + 10));
            let (time, date) = (u16le(b, p + 12), u16le(b, p + 14));
            let crc = u32le(b, p + 16);
            let mut csize = u64::from(u32le(b, p + 20));
            let mut usize_ = u64::from(u32le(b, p + 24));
            let (nlen, elen, clen) = (usize::from(u16le(b, p + 28)), usize::from(u16le(b, p + 30)), usize::from(u16le(b, p + 32)));
            let attr = u32le(b, p + 38);
            let mut lho = u64::from(u32le(b, p + 42));
            let end = p + 46 + nlen + elen + clen;
            if end > b.len() {
                return Err(Error::new(ER_NOZIP));
            }
            let name = b[p + 46..p + 46 + nlen].to_vec();
            let extra = b[p + 46 + nlen..p + 46 + nlen + elen].to_vec();
            let ecomment = b[p + 46 + nlen + elen..end].to_vec();
            let mut mtime = dos_to_unix(time, date);
            // Extra fields: zip64 sizes, extended timestamp.
            let mut e = 0;
            while e + 4 <= extra.len() {
                let (id, len) = (u16le(&extra, e), usize::from(u16le(&extra, e + 2)));
                let body = &extra[(e + 4).min(extra.len())..(e + 4 + len).min(extra.len())];
                if id == 1 {
                    let mut q = 0;
                    for field in [&mut usize_, &mut csize] {
                        if *field == 0xFFFF_FFFF && q + 8 <= body.len() {
                            *field = u64le(body, q);
                            q += 8;
                        }
                    }
                    if lho == 0xFFFF_FFFF && q + 8 <= body.len() {
                        lho = u64le(body, q);
                    }
                } else if id == 0x5455 && body.len() >= 5 && body[0] & 1 != 0 {
                    mtime = i64::from(u32le(body, 1));
                }
                e += 4 + len;
            }
            let lho = (lho as i64 + shift) as usize;
            if checkcons {
                if lho + 30 > b.len() || b[lho..lho + 4] != [0x50, 0x4b, 3, 4] {
                    return Err(Error::new(ER_INCONS));
                }
                let (lnlen, lelen) = (usize::from(u16le(b, lho + 26)), usize::from(u16le(b, lho + 28)));
                let data = lho + 30 + lnlen + lelen;
                if lho + 30 + lnlen > b.len() || b[lho + 30..lho + 30 + lnlen] != name[..] || (data as u64 + csize) > cd_off + shift as u64 {
                    return Err(Error::new(ER_INCONS));
                }
            }
            let rec = Rec {
                name,
                comment: ecomment,
                method,
                level: 0,
                crc,
                size: usize_,
                comp_size: csize,
                mtime,
                opsys: (made >> 8) as u8,
                attr,
                flags,
                extra_local: Vec::new(),
                extra_central: extra.to_vec(),
                source: Source::Raw(Rc::clone(&raw), lho),
                data_changed: false,
                recompress: false,
                src_method: method,
                encrypted: flags & 1 != 0,
            };
            slots.push(Slot { orig: Some(rec.clone()), cur: rec, deleted: false });
            p = end;
        }
        let cd_bytes = &b[(cd_off as i64 + shift) as usize..((cd_off as i64 + shift) as usize + cd_size as usize).min(b.len())];
        let torrent = comment.len() == 22
            && comment.starts_with(b"TORRENTZIPPED-")
            && std::str::from_utf8(&comment[14..]).ok().and_then(|h| u32::from_str_radix(h, 16).ok()) == Some(crc32fast::hash(cd_bytes));
        Ok(Archive { slots, comment: comment.clone(), orig_comment: comment, readonly: false, password: None, torrent, want_torrent: false })
    }

    // ---- lookup ----

    pub fn count(&self) -> usize {
        self.slots.len()
    }

    /// The entry's record: its original (`unchanged`) or its current state.
    fn rec(&self, i: usize, unchanged: bool) -> Result<&Rec, Error> {
        let s = self.slots.get(i).ok_or(Error::new(ER_INVAL))?;
        if unchanged {
            return s.orig.as_ref().ok_or(Error::new(ER_INVAL));
        }
        if s.deleted {
            return Err(Error::new(ER_DELETED));
        }
        Ok(&s.cur)
    }

    pub fn name(&self, i: usize, unchanged: bool) -> Result<&[u8], Error> {
        self.rec(i, unchanged).map(|r| &r.name[..])
    }

    pub fn locate(&self, name: &[u8], nocase: bool, nodir: bool, unchanged: bool) -> Option<usize> {
        let norm = |n: &[u8]| -> Vec<u8> {
            let n = if nodir { n.rsplit(|&c| c == b'/').find(|s| !s.is_empty()).map(|_| basename(n)).unwrap_or(n) } else { n };
            if nocase { n.to_ascii_lowercase() } else { n.to_vec() }
        };
        let want = norm(name);
        for i in 0..self.slots.len() {
            let Ok(r) = self.rec(i, unchanged) else { continue };
            if norm(&r.name) == want {
                return Some(i);
            }
        }
        None
    }

    pub fn stat(&self, i: usize, unchanged: bool) -> Result<Stat, Error> {
        let r = self.rec(i, unchanged)?;
        let (size, comp_size) = match &r.source {
            Source::Raw(..) if !r.data_changed => (r.size, r.comp_size as i64),
            Source::Memory(d) => (d.len() as u64, d.len() as i64),
            Source::File { path, start, len, .. } => {
                let total = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                let avail = total.saturating_sub(*start);
                let size = if *len > 0 {
                    avail.min(*len as u64)
                } else if *len == -2 {
                    0
                } else {
                    avail
                };
                (size, if r.method == CM_STORE { size as i64 } else { 0 })
            }
            Source::Raw(..) => (r.size, r.size as i64),
        };
        // What libzip's sources report: a pending entry is still uncompressed.
        let comp_method = if r.method == CM_DEFAULT && !matches!(r.source, Source::Raw(..)) { CM_STORE } else { r.method };
        Ok(Stat {
            name: r.name.clone(),
            index: i,
            crc: if r.data_changed { 0 } else { r.crc },
            size,
            mtime: r.mtime,
            comp_size,
            comp_method,
            encryption: 0,
        })
    }

    // ---- data ----

    /// The entry's uncompressed bytes. libzip reads an entry whose data is a
    /// file still to be added no more than one in memory: "Entry has been changed".
    pub fn read(&self, i: usize, unchanged: bool) -> Result<Vec<u8>, Error> {
        let r = self.rec(i, unchanged)?;
        if matches!(r.source, Source::File { .. }) {
            return Err(Error::new(ER_CHANGED));
        }
        self.read_rec(r)
    }

    /// The entry's bytes as stored in the archive (ZipArchive::FL_COMPRESSED).
    pub fn read_compressed(&self, i: usize, unchanged: bool) -> Result<Vec<u8>, Error> {
        let r = self.rec(i, unchanged)?;
        match &r.source {
            Source::Raw(raw, lho) if !r.data_changed => Ok(raw_data(raw, *lho, r.comp_size)?.0.to_vec()),
            Source::File { .. } => Err(Error::new(ER_CHANGED)),
            _ => self.read_rec(r),
        }
    }

    fn read_rec(&self, r: &Rec) -> Result<Vec<u8>, Error> {
        match &r.source {
            Source::Memory(d) => Ok(d.to_vec()),
            Source::File { loaded: Some(d), .. } => Ok(d.to_vec()),
            Source::File { path, start, len, .. } => {
                let data = std::fs::read(path).map_err(|e| Error::io(if e.kind() == std::io::ErrorKind::NotFound { ER_OPEN } else { ER_READ }, &e))?;
                let from = (*start as usize).min(data.len());
                let to = if *len > 0 { (from + *len as usize).min(data.len()) } else { data.len() };
                Ok(data[from..to].to_vec())
            }
            Source::Raw(raw, lho) => {
                if *lho + 30 <= raw.len() && raw[*lho..*lho + 4] == [0x50, 0x4b, 3, 4] && u16le(raw, *lho + 6) & 8 == 0 {
                    let (lc, ls) = (u64::from(u32le(raw, *lho + 18)), u64::from(u32le(raw, *lho + 22)));
                    if (lc != 0xFFFF_FFFF && lc != r.comp_size) || (ls != 0xFFFF_FFFF && ls != r.size) {
                        return Err(Error::new(ER_INCONS));
                    }
                }
                let (comp, _) = raw_data(raw, *lho, r.comp_size)?;
                if r.encrypted {
                    return Err(Error::new(if r.src_method == 99 { ER_ENCRNOTSUPP } else { ER_NOPASSWD }));
                }
                let out = match r.src_method {
                    CM_STORE => comp.to_vec(),
                    CM_DEFLATE => miniz_oxide::inflate::decompress_to_vec_with_limit(comp, (r.size as usize).saturating_add(1) + 64)
                        .map_err(|_| Error::new(ER_COMPRESSED_DATA))?,
                    _ => return Err(Error::new(ER_COMPNOTSUPP)),
                };
                if out.len() as u64 != r.size {
                    return Err(Error::new(ER_COMPRESSED_DATA));
                }
                if crc32fast::hash(&out) != r.crc {
                    return Err(Error::new(ER_CRC));
                }
                Ok(out)
            }
        }
    }

    // ---- changes ----

    /// A new entry (or, with `replace`, a replacement of entry `at`).
    pub fn set_data(&mut self, at: Option<usize>, name: Vec<u8>, source: Source) -> usize {
        match at {
            Some(i) => {
                let s = &mut self.slots[i];
                let old = std::mem::replace(&mut s.cur, Rec::new(name, source));
                s.cur.comment = old.comment;
                s.cur.opsys = old.opsys;
                s.cur.attr = old.attr;
                s.cur.method = old.method;
                s.cur.level = old.level;
                s.deleted = false;
                i
            }
            None => {
                let rec = Rec::new(name, source);
                self.slots.push(Slot { orig: None, cur: rec, deleted: false });
                self.slots.len() - 1
            }
        }
    }

    pub fn unchange(&mut self, i: usize) {
        let s = &mut self.slots[i];
        match &s.orig {
            Some(o) => {
                s.cur = o.clone();
                s.deleted = false;
            }
            None => s.deleted = true,
        }
    }

    /// Whether closing the archive would change the file.
    pub fn is_changed(&self) -> bool {
        if self.comment != self.orig_comment || self.want_torrent != self.torrent {
            return true;
        }
        self.slots.iter().any(|s| match (&s.orig, s.deleted) {
            (None, deleted) => !deleted,
            (Some(_), true) => true,
            (Some(o), false) => {
                let c = &s.cur;
                c.name != o.name
                    || c.comment != o.comment
                    || c.mtime != o.mtime
                    || c.opsys != o.opsys
                    || c.attr != o.attr
                    || c.data_changed
                    || c.recompress
            }
        })
    }

    // ---- writing ----

    /// The archive's bytes.
    pub fn build(&self) -> Result<Vec<u8>, Error> {
        let mut out: Vec<u8> = Vec::new();
        let mut central: Vec<u8> = Vec::new();
        let mut count = 0u64;
        let mut live: Vec<&Slot> = self.slots.iter().filter(|s| !s.deleted).collect();
        if self.want_torrent {
            live.sort_by_key(|s| s.cur.name.to_ascii_lowercase());
        }
        for s in live {
            let torrent_rec;
            let r = if self.want_torrent {
                // A torrentzip: one fixed time, deflate at the top level, nothing else.
                let mut t = s.cur.clone();
                t.mtime = dos_to_unix(0xBC00, 0x2198);
                t.method = CM_DEFLATE;
                t.level = 9;
                t.recompress = true;
                t.opsys = 0;
                t.attr = 0;
                t.comment = Vec::new();
                t.extra_central = Vec::new();
                t.extra_local = Vec::new();
                torrent_rec = t;
                &torrent_rec
            } else {
                &s.cur
            };
            let offset = out.len() as u64;
            let (method, comp, size, crc) = self.encode(r)?;
            let name = &r.name;
            let utf8 = r.flags & 0x800 != 0 || (!name.is_ascii() && std::str::from_utf8(name).is_ok());
            let level_bits = if method == 8 { match r.level { 1 => 6, 2 | 3 => 4, _ => 2 } } else { 0 };
            let flags = (r.flags & !0x080F) | if utf8 { 0x800 } else { 0 } | u16::from(r.encrypted) | level_bits;
            let (time, date) = unix_to_dos(r.mtime);
            let big = size >= 0xFFFF_FFFF || comp.len() as u64 >= 0xFFFF_FFFF || offset >= 0xFFFF_FFFF;
            let needed: u16 = if big { 45 } else if method == 8 || name.ends_with(b"/") { 20 } else { 10 };
            // local header
            out.extend_from_slice(&[0x50, 0x4b, 3, 4]);
            out.extend_from_slice(&needed.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&(method as u16).to_le_bytes());
            out.extend_from_slice(&time.to_le_bytes());
            out.extend_from_slice(&date.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            let (l_csize, l_size) = if big { (0xFFFF_FFFFu32, 0xFFFF_FFFFu32) } else { (comp.len() as u32, size as u32) };
            out.extend_from_slice(&l_csize.to_le_bytes());
            out.extend_from_slice(&l_size.to_le_bytes());
            let mut lextra = r.extra_local.clone();
            if big {
                lextra.extend_from_slice(&1u16.to_le_bytes());
                lextra.extend_from_slice(&16u16.to_le_bytes());
                lextra.extend_from_slice(&size.to_le_bytes());
                lextra.extend_from_slice(&(comp.len() as u64).to_le_bytes());
            }
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&(lextra.len() as u16).to_le_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(&lextra);
            out.extend_from_slice(&comp);
            // central directory record
            central.extend_from_slice(&[0x50, 0x4b, 1, 2]);
            central.extend_from_slice(&((u16::from(r.opsys) << 8) | 63).to_le_bytes());
            central.extend_from_slice(&needed.to_le_bytes());
            central.extend_from_slice(&flags.to_le_bytes());
            central.extend_from_slice(&(method as u16).to_le_bytes());
            central.extend_from_slice(&time.to_le_bytes());
            central.extend_from_slice(&date.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&l_csize.to_le_bytes());
            central.extend_from_slice(&l_size.to_le_bytes());
            let mut cextra: Vec<u8> = strip_zip64(&r.extra_central);
            if big {
                let mut z = Vec::new();
                z.extend_from_slice(&size.to_le_bytes());
                z.extend_from_slice(&(comp.len() as u64).to_le_bytes());
                if offset >= 0xFFFF_FFFF {
                    z.extend_from_slice(&offset.to_le_bytes());
                }
                cextra.extend_from_slice(&1u16.to_le_bytes());
                cextra.extend_from_slice(&(z.len() as u16).to_le_bytes());
                cextra.extend_from_slice(&z);
            }
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&(cextra.len() as u16).to_le_bytes());
            central.extend_from_slice(&(r.comment.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&r.attr.to_le_bytes());
            central.extend_from_slice(&(if offset >= 0xFFFF_FFFF { 0xFFFF_FFFFu32 } else { offset as u32 }).to_le_bytes());
            central.extend_from_slice(name);
            central.extend_from_slice(&cextra);
            central.extend_from_slice(&r.comment);
            count += 1;
        }
        let cd_off = out.len() as u64;
        out.extend_from_slice(&central);
        let torrent_comment = format!("TORRENTZIPPED-{:08X}", crc32fast::hash(&central)).into_bytes();
        let comment: &[u8] = if self.want_torrent { &torrent_comment } else { &self.comment };
        let cd_size = central.len() as u64;
        if count >= 0xFFFF || cd_off >= 0xFFFF_FFFF || cd_size >= 0xFFFF_FFFF {
            let at = out.len() as u64;
            out.extend_from_slice(&[0x50, 0x4b, 6, 6]);
            out.extend_from_slice(&44u64.to_le_bytes());
            out.extend_from_slice(&((3u16 << 8) | 63).to_le_bytes());
            out.extend_from_slice(&45u16.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&cd_size.to_le_bytes());
            out.extend_from_slice(&cd_off.to_le_bytes());
            out.extend_from_slice(&[0x50, 0x4b, 6, 7]);
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&at.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
        }
        out.extend_from_slice(&[0x50, 0x4b, 5, 6, 0, 0, 0, 0]);
        out.extend_from_slice(&(count.min(0xFFFF) as u16).to_le_bytes());
        out.extend_from_slice(&(count.min(0xFFFF) as u16).to_le_bytes());
        out.extend_from_slice(&(cd_size.min(0xFFFF_FFFF) as u32).to_le_bytes());
        out.extend_from_slice(&(cd_off.min(0xFFFF_FFFF) as u32).to_le_bytes());
        out.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        out.extend_from_slice(comment);
        Ok(out)
    }

    /// The bytes an entry is written with: method, compressed data, size, crc.
    fn encode(&self, r: &Rec) -> Result<(i32, Vec<u8>, u64, u32), Error> {
        if let Source::Raw(raw, lho) = &r.source
            && !r.data_changed && !r.recompress {
                let (comp, _) = raw_data(raw, *lho, r.comp_size)?;
                return Ok((r.src_method, comp.to_vec(), r.size, r.crc));
            }
        let data = self.read_rec(r)?;
        let crc = crc32fast::hash(&data);
        let level = if r.level == 0 { 6 } else { r.level.min(9) as u8 };
        let is_dir = r.name.ends_with(b"/");
        let want = if r.method == CM_DEFAULT { CM_DEFLATE } else { r.method };
        if want == CM_DEFLATE && ((!data.is_empty() && !is_dir) || self.want_torrent) {
            let comp = miniz_oxide::deflate::compress_to_vec(&data, level);
            if r.method != CM_DEFAULT || comp.len() < data.len() {
                return Ok((CM_DEFLATE, comp, data.len() as u64, crc));
            }
        }
        let size = data.len() as u64;
        Ok((CM_STORE, data, size, crc))
    }
}

fn strip_zip64(extra: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = 0;
    while e + 4 <= extra.len() {
        let (id, len) = (u16le(extra, e), usize::from(u16le(extra, e + 2)));
        let end = (e + 4 + len).min(extra.len());
        if id != 1 {
            out.extend_from_slice(&extra[e..end]);
        }
        e = end;
    }
    out
}

pub fn basename(n: &[u8]) -> &[u8] {
    let trimmed = if n.ends_with(b"/") && n.len() > 1 { &n[..n.len() - 1] } else { n };
    match trimmed.iter().rposition(|&c| c == b'/') {
        Some(p) => &trimmed[p + 1..],
        None => trimmed,
    }
}

/// An entry's compressed bytes in the archive as read, and where they start.
fn raw_data(raw: &[u8], lho: usize, comp_size: u64) -> Result<(&[u8], usize), Error> {
    if lho + 30 > raw.len() || raw[lho..lho + 4] != [0x50, 0x4b, 3, 4] {
        return Err(Error::new(ER_INCONS));
    }
    let start = lho + 30 + usize::from(u16le(raw, lho + 26)) + usize::from(u16le(raw, lho + 28));
    let end = start.checked_add(comp_size as usize).filter(|&e| e <= raw.len()).ok_or(Error::new(ER_TRUNCATED_ZIP))?;
    Ok((&raw[start..end], start))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(entries: &[(&str, &[u8])]) -> Archive {
        let mut a = Archive::new();
        for (name, data) in entries {
            a.set_data(None, name.as_bytes().to_vec(), Source::Memory(Rc::new(data.to_vec())));
        }
        a
    }

    #[test]
    fn written_archive_reads_back_with_checks_on() {
        let a = archive(&[("a.txt", &b"alpha ".repeat(100)), ("empty", b""), ("dir/", b"")]);
        let back = Archive::parse(a.build().unwrap(), true).unwrap();
        assert_eq!(back.count(), 3);
        assert_eq!(back.name(0, false).unwrap(), b"a.txt");
        assert_eq!(back.read(0, false).unwrap(), b"alpha ".repeat(100));
        assert_eq!(back.read(1, false).unwrap(), b"");
        let st = back.stat(0, false).unwrap();
        assert_eq!((st.size, st.comp_method), (600, CM_DEFLATE));
        assert!(st.comp_size < 600);
        // Tiny data is stored: deflate would make it longer.
        let tiny = Archive::parse(archive(&[("t", b"x")]).build().unwrap(), true).unwrap();
        assert_eq!(tiny.stat(0, false).unwrap().comp_method, CM_STORE);
    }

    #[test]
    fn a_flipped_byte_is_a_crc_error() {
        let mut a = archive(&[("s", b"stored data")]);
        a.slots[0].cur.method = CM_STORE;
        let mut bytes = a.build().unwrap();
        let at = bytes.windows(11).position(|w| w == b"stored data").unwrap();
        bytes[at] ^= 1;
        let back = Archive::parse(bytes, false).unwrap();
        assert_eq!(back.read(0, false).unwrap_err().code, ER_CRC);
    }

    #[test]
    fn only_real_changes_count() {
        let a = archive(&[("a", b"1"), ("b", b"2")]);
        let mut back = Archive::parse(a.build().unwrap(), false).unwrap();
        assert!(!back.is_changed());
        back.slots[0].cur.name = b"renamed".to_vec();
        back.slots[1].cur.comment = b"note".to_vec();
        assert!(back.is_changed());
        back.unchange(0);
        back.unchange(1);
        assert!(!back.is_changed());
        back.slots[0].deleted = true;
        assert!(back.is_changed());
        back.unchange(0);
        assert!(!back.is_changed());
    }

    #[test]
    fn torrentzip_is_written_and_recognised() {
        let mut a = archive(&[("B", b"b"), ("a", b"a")]);
        a.want_torrent = true;
        let back = Archive::parse(a.build().unwrap(), true).unwrap();
        assert!(back.torrent);
        assert_eq!((back.name(0, false).unwrap(), back.name(1, false).unwrap()), (&b"a"[..], &b"B"[..]));
        assert_eq!(back.stat(0, false).unwrap().mtime, dos_to_unix(0xBC00, 0x2198));
    }

    #[test]
    fn dos_times_round_trip_to_two_seconds() {
        for t in [315_532_800, 1_700_000_000, 4_102_444_798] {
            let (time, date) = unix_to_dos(t);
            let back = dos_to_unix(time, date);
            assert!((t - back).abs() <= 1, "{t} -> {back}");
        }
    }

    #[test]
    fn not_a_zip_and_a_directory_past_the_end_are_refused() {
        assert_eq!(Archive::parse(vec![0; 100], false).err().map(|e| e.code), Some(ER_NOZIP));
        let mut bytes = archive(&[("a", b"1")]).build().unwrap();
        let n = bytes.len();
        // The central directory size grown by one reaches past the end record.
        let size = u32::from_le_bytes([bytes[n - 10], bytes[n - 9], bytes[n - 8], bytes[n - 7]]) + 1;
        bytes[n - 10..n - 6].copy_from_slice(&size.to_le_bytes());
        assert_eq!(Archive::parse(bytes, false).err().map(|e| e.code), Some(ER_INCONS));
    }
}
