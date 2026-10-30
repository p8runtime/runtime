//! X.509 certificates: reading them (DER, PEM), and what
//! openssl_x509_parse() reports of one: names as X509_NAME_oneline and
//! X509_NAME_print_ex write them, the subject hash, the purposes
//! (X509_check_purpose, as OpenSSL 3.5 decides them) and the extensions as
//! X509V3_EXT_print prints them.

use crate::der::{self, Reader, Tlv};
use crate::digest::Md;
use crate::oids;
use crate::pkey::Key;

pub struct Entry {
    pub oid: String,
    pub tag: u8,
    pub value: Vec<u8>,
    /// Which RDN (SET) of the name it is in.
    pub set: usize,
}

pub struct Name {
    pub entries: Vec<Entry>,
}

pub struct Ext {
    pub oid: String,
    pub critical: bool,
    pub value: Vec<u8>,
}

pub struct Cert {
    pub der: Vec<u8>,
    pub tbs: Vec<u8>,
    pub version: i64,
    /// The serial number INTEGER's contents (two's complement).
    pub serial: Vec<u8>,
    pub issuer: Name,
    pub subject: Name,
    pub not_before: (u8, Vec<u8>),
    pub not_after: (u8, Vec<u8>),
    pub spki: Vec<u8>,
    pub exts: Vec<Ext>,
    pub sig_oid: String,
    pub signature: Vec<u8>,
}

fn parse_name(t: &Tlv) -> Option<Name> {
    let mut entries = Vec::new();
    let mut sets = t.reader();
    let mut set = 0;
    while let Some(s) = sets.read() {
        if s.tag != der::SET {
            return None;
        }
        let mut atvs = s.reader();
        while let Some(atv) = atvs.read() {
            let mut r = atv.reader();
            let oid = der::oid_text(r.expect(der::OID)?.body);
            let v = r.read()?;
            entries.push(Entry { oid, tag: v.tag, value: v.body.to_vec(), set });
        }
        set += 1;
    }
    Some(Name { entries })
}

impl Cert {
    pub fn from_der(data: &[u8]) -> Option<Cert> {
        let mut outer = Reader::new(data);
        let cert = outer.expect(der::SEQUENCE)?;
        let mut c = cert.reader();
        let tbs = c.expect(der::SEQUENCE)?;
        let sig_alg = c.expect(der::SEQUENCE)?;
        let sig = c.expect(der::BIT_STRING)?;
        let mut t = tbs.reader();
        let version = match t.optional(0xa0) {
            Some(v) => {
                let i = der::parse_one(v.body).filter(|i| i.tag == der::INTEGER)?;
                i.body.iter().fold(0i64, |a, b| (a << 8) | i64::from(*b))
            }
            None => 0,
        };
        let serial = t.expect(der::INTEGER)?.body.to_vec();
        t.expect(der::SEQUENCE)?;
        let issuer = parse_name(&t.expect(der::SEQUENCE)?)?;
        let validity = t.expect(der::SEQUENCE)?;
        let mut v = validity.reader();
        let nb = v.read()?;
        let na = v.read()?;
        if !time_well_formed(&nb) || !time_well_formed(&na) {
            return None;
        }
        let subject = parse_name(&t.expect(der::SEQUENCE)?)?;
        let spki = t.expect(der::SEQUENCE)?.raw.to_vec();
        let mut exts = Vec::new();
        while let Some(x) = t.read() {
            if x.tag != 0xa3 {
                continue;
            }
            let list = der::parse_one(x.body).filter(|l| l.tag == der::SEQUENCE)?;
            let mut l = list.reader();
            while let Some(e) = l.read() {
                let mut er = e.reader();
                let oid = der::oid_text(er.expect(der::OID)?.body);
                let critical = er.optional(0x01).is_some_and(|b| b.body.first().is_some_and(|v| *v != 0));
                let value = er.expect(der::OCTET_STRING)?.body.to_vec();
                exts.push(Ext { oid, critical, value });
            }
        }
        let sig_oid = der::oid_text(sig_alg.reader().expect(der::OID)?.body);
        Some(Cert {
            der: cert.raw.to_vec(),
            tbs: tbs.raw.to_vec(),
            version,
            serial,
            issuer,
            subject,
            not_before: (nb.tag, nb.body.to_vec()),
            not_after: (na.tag, na.body.to_vec()),
            spki,
            exts,
            sig_oid,
            signature: sig.body.get(1..).unwrap_or_default().to_vec(),
        })
    }

    /// The first certificate in PEM `text` (PEM_read_bio_X509).
    pub fn from_pem(text: &[u8]) -> Option<Cert> {
        let pem = der::pem_find(text, &["CERTIFICATE", "X509 CERTIFICATE"])?;
        Cert::from_der(&pem.der)
    }

    pub fn pem(&self) -> String {
        der::pem_encode("CERTIFICATE", &self.der)
    }

    pub fn public_key(&self) -> Option<Key> {
        Key::from_spki(&self.spki).ok()
    }

    pub fn ext(&self, oid: &str) -> Option<&Ext> {
        self.exts.iter().find(|e| e.oid == oid)
    }

    /// X509_verify: whether `key` made the certificate's signature.
    pub fn verify(&self, key: &Key) -> i64 {
        let Some((md, kind)) = sig_alg(&self.sig_oid) else { return -1 };
        let matches = matches!((kind, key), ("rsa", Key::Rsa { .. }) | ("ec", Key::Ec { .. }) | ("ed25519", Key::Ed25519 { .. }));
        if !matches {
            return -1;
        }
        match key.verify(md, &self.tbs, &self.signature) {
            1 => 1,
            _ => 0,
        }
    }
}

/// Whether a validity time is one OpenSSL 3.3+ reads: a UTCTime or a
/// GeneralizedTime starting with every field down to the seconds.
fn time_well_formed(t: &Tlv) -> bool {
    let digits = match t.tag {
        der::UTC_TIME => 12,
        der::GENERALIZED_TIME => 14,
        _ => return false,
    };
    t.body.len() > digits && t.body[..digits].iter().all(u8::is_ascii_digit) && matches!(t.body[digits], b'Z' | b'+' | b'-' | b'.')
}

/// A signature algorithm's digest and key type.
fn sig_alg(oid: &str) -> Option<(Option<Md>, &'static str)> {
    Some(match oid {
        "1.2.840.113549.1.1.3" => (Some(Md::Md4), "rsa"),
        "1.2.840.113549.1.1.4" => (Some(Md::Md5), "rsa"),
        "1.2.840.113549.1.1.5" | "1.3.14.3.2.29" => (Some(Md::Sha1), "rsa"),
        "1.2.840.113549.1.1.11" => (Some(Md::Sha256), "rsa"),
        "1.2.840.113549.1.1.12" => (Some(Md::Sha384), "rsa"),
        "1.2.840.113549.1.1.13" => (Some(Md::Sha512), "rsa"),
        "1.2.840.113549.1.1.14" => (Some(Md::Sha224), "rsa"),
        "1.2.840.10045.4.1" => (Some(Md::Sha1), "ec"),
        "1.2.840.10045.4.3.1" => (Some(Md::Sha224), "ec"),
        "1.2.840.10045.4.3.2" => (Some(Md::Sha256), "ec"),
        "1.2.840.10045.4.3.3" => (Some(Md::Sha384), "ec"),
        "1.2.840.10045.4.3.4" => (Some(Md::Sha512), "ec"),
        "1.3.101.112" => (None, "ed25519"),
        _ => return None,
    })
}

// ---- names ----

/// ASN1_STRING_to_UTF8: a string value as UTF-8.
pub fn to_utf8(tag: u8, value: &[u8]) -> Vec<u8> {
    match tag {
        der::BMP_STRING => {
            let units: Vec<u16> = value.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
            String::from_utf16_lossy(&units).into_bytes()
        }
        der::UNIVERSAL_STRING => value
            .chunks(4)
            .filter_map(|c| (c.len() == 4).then(|| char::from_u32(u32::from_be_bytes([c[0], c[1], c[2], c[3]]))).flatten())
            .collect::<String>()
            .into_bytes(),
        der::UTF8_STRING => value.to_vec(),
        // Latin-1 (T61String, and anything else byte-wide)
        _ if value.is_ascii() => value.to_vec(),
        _ => value.iter().map(|b| char::from(*b)).collect::<String>().into_bytes(),
    }
}

/// The short name OpenSSL gives an attribute (OBJ_nid2sn), "UNDEF" for one it does not know.
fn attr_sn(oid: &str) -> &'static str {
    oids::by_oid(oid).map_or("UNDEF", |o| o.sn)
}

fn attr_ln(oid: &str) -> &'static str {
    oids::by_oid(oid).map_or("undefined", |o| o.ln)
}

impl Name {
    /// X509_NAME_oneline
    pub fn oneline(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for e in &self.entries {
            out.push(b'/');
            match oids::by_oid(&e.oid) {
                Some(o) => out.extend_from_slice(o.sn.as_bytes()),
                None => out.extend_from_slice(e.oid.as_bytes()),
            }
            out.push(b'=');
            let mut keep = [true; 4];
            if e.tag == 0x1b && e.value.len() % 4 == 0 {
                let mut doit = [false; 4];
                for (j, b) in e.value.iter().enumerate() {
                    if *b != 0 {
                        doit[j & 3] = true;
                    }
                }
                keep = if doit[0] | doit[1] | doit[2] { [true; 4] } else { [false, false, false, true] };
            }
            for (j, &c) in e.value.iter().enumerate() {
                if !keep[j & 3] {
                    continue;
                }
                if !(b' '..=b'~').contains(&c) {
                    out.extend_from_slice(format!("\\x{c:02X}").as_bytes());
                } else if c == b'/' || c == b'+' {
                    out.push(b'\\');
                    out.push(c);
                } else {
                    out.push(c);
                }
            }
        }
        out
    }

    /// (key, UTF-8 value) pairs as php_openssl_add_assoc_name_entry reads them.
    pub fn pairs(&self, short: bool) -> Vec<(&'static str, Vec<u8>)> {
        self.entries.iter().map(|e| (if short { attr_sn(&e.oid) } else { attr_ln(&e.oid) }, to_utf8(e.tag, &e.value))).collect()
    }

    /// X509_NAME_print_ex(XN_FLAG_ONELINE): "C = US, O = Example".
    pub fn print_oneline(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            if i > 0 {
                out.extend_from_slice(if e.set == self.entries[i - 1].set { b" + " } else { b", " });
            }
            match oids::by_oid(&e.oid) {
                Some(o) => out.extend_from_slice(o.sn.as_bytes()),
                None => out.extend_from_slice(e.oid.as_bytes()),
            }
            out.extend_from_slice(b" = ");
            out.extend(rfc2253_value(e.tag, &e.value));
        }
        out
    }

    /// The canonical encoding X509_NAME_hash_ex hashes.
    fn canon(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let sets = self.entries.iter().map(|e| e.set).max().map_or(0, |m| m + 1);
        for s in 0..sets {
            let mut items: Vec<Vec<u8>> = self
                .entries
                .iter()
                .filter(|e| e.set == s)
                .map(|e| {
                    let canon = matches!(e.tag, der::UTF8_STRING | der::BMP_STRING | der::UNIVERSAL_STRING | der::PRINTABLE_STRING | der::T61_STRING | der::IA5_STRING | der::VISIBLE_STRING);
                    let value = if canon { der::tlv(der::UTF8_STRING, &canon_value(&to_utf8(e.tag, &e.value))) } else { der::tlv(e.tag, &e.value) };
                    der::seq(&[&der::oid(&e.oid), &value])
                })
                .collect();
            items.sort();
            out.extend(der::tlv(der::SET, &items.concat()));
        }
        out
    }

    /// X509_NAME_hash_ex: what CA directories name a certificate by.
    pub fn hash(&self) -> u32 {
        let h = Md::Sha1.hash(&self.canon());
        u32::from_le_bytes([h[0], h[1], h[2], h[3]])
    }

    pub fn same(&self, other: &Name) -> bool {
        self.canon() == other.canon()
    }
}

fn canon_value(v: &[u8]) -> Vec<u8> {
    let space = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r');
    let start = v.iter().position(|c| !space(*c)).unwrap_or(v.len());
    let end = v.iter().rposition(|c| !space(*c)).map_or(start, |e| e + 1);
    let v = &v[start..end.max(start)];
    let mut out = Vec::with_capacity(v.len());
    let mut i = 0;
    while i < v.len() {
        let c = v[i];
        if c >= 0x80 {
            out.push(c);
            i += 1;
        } else if space(c) {
            out.push(b' ');
            while i < v.len() && space(v[i]) {
                i += 1;
            }
        } else {
            out.push(c.to_ascii_lowercase());
            i += 1;
        }
    }
    out
}

/// A value as ASN1_STRFLGS_RFC2253 | ASN1_STRFLGS_ESC_QUOTE prints it.
fn rfc2253_value(tag: u8, value: &[u8]) -> Vec<u8> {
    let utf8 = to_utf8(tag, value);
    let special = |c: u8| matches!(c, b',' | b'+' | b'"' | b'\\' | b'<' | b'>' | b';');
    let quote = utf8.iter().any(|c| special(*c));
    let mut out = Vec::new();
    if quote {
        out.push(b'"');
    }
    let n = utf8.len();
    for (i, &c) in utf8.iter().enumerate() {
        if !(0x20..0x7f).contains(&c) {
            out.extend_from_slice(format!("\\{c:02X}").as_bytes());
        } else if quote {
            if c == b'"' || c == b'\\' {
                out.push(b'\\');
            }
            out.push(c);
        } else if (i == 0 && (c == b'#' || c == b' ')) || (i + 1 == n && c == b' ') {
            out.push(b'\\');
            out.push(c);
        } else {
            out.push(c);
        }
    }
    if quote {
        out.push(b'"');
    }
    out
}

// ---- times ----

/// php_openssl_asn1_time_to_time_t: Err(the warning) when PHP warns and gives -1.
pub fn time_t(tag: u8, s: &[u8]) -> Result<i64, String> {
    if tag != der::UTC_TIME && tag != der::GENERALIZED_TIME {
        return Err("Illegal ASN1 data type for timestamp".into());
    }
    if s.contains(&0) {
        return Err("Illegal length in timestamp".into());
    }
    if s.len() < 13 || (tag == der::GENERALIZED_TIME && s.len() < 15) {
        return Err(format!("Unable to parse time string {} correctly", String::from_utf8_lossy(s)));
    }
    let num = |b: &[u8]| -> i64 { std::str::from_utf8(b).ok().and_then(|t| t.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok()).unwrap_or(0) };
    let n = s.len();
    let sec = num(&s[n - 3..n - 1]);
    let min = num(&s[n - 5..n - 3]);
    let hour = num(&s[n - 7..n - 5]);
    let day = num(&s[n - 9..n - 7]);
    let mon = num(&s[n - 11..n - 9]);
    let year = if tag == der::UTC_TIME {
        let y = num(&s[n - 13..n - 11]);
        if y < 68 { 2000 + y } else { 1900 + y }
    } else {
        num(&s[n - 15..n - 11])
    };
    Ok(days_from_civil(year, mon, day) * 86400 + hour * 3600 + min * 60 + sec)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    // Months out of range carry into the year, as mktime() normalizes them.
    let (y, m) = (y + (m - 1).div_euclid(12), (m - 1).rem_euclid(12) + 1);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

// ---- serial numbers ----

/// (decimal, hex) as i2s_ASN1_INTEGER and BN_bn2hex give them; a number
/// of 128 bits or more is "0x" and hex in the first, as OpenSSL 3 prints it.
pub fn serial_strings(body: &[u8]) -> (String, String) {
    let negative = body.first().is_some_and(|b| b & 0x80 != 0);
    let magnitude = if negative {
        // two's complement: invert and add one
        let mut m: Vec<u8> = body.iter().map(|b| !b).collect();
        for b in m.iter_mut().rev() {
            *b = b.wrapping_add(1);
            if *b != 0 {
                break;
            }
        }
        m
    } else {
        body.to_vec()
    };
    let n = rsa::BigUint::from_bytes_be(&magnitude);
    let sign = if negative { "-" } else { "" };
    let bytes = crate::pkey::minimal(&magnitude);
    let hex = if bytes.is_empty() { "0".to_string() } else { der::hex_upper(&bytes, "") };
    let dec = if n.bits() >= 128 { format!("{sign}0x{hex}") } else { format!("{sign}{}", n.to_str_radix(10)) };
    (dec, format!("{sign}{hex}"))
}

// ---- purposes (OpenSSL 3.5's v3_purp.c) ----

const KU_DIGITAL_SIGNATURE: u32 = 0x80;
const KU_NON_REPUDIATION: u32 = 0x40;
const KU_KEY_ENCIPHERMENT: u32 = 0x20;
const KU_KEY_AGREEMENT: u32 = 0x08;
const KU_KEY_CERT_SIGN: u32 = 0x04;
const KU_CRL_SIGN: u32 = 0x02;

const XKU_SSL_SERVER: u32 = 0x1;
const XKU_SSL_CLIENT: u32 = 0x2;
const XKU_SMIME: u32 = 0x4;
const XKU_CODE_SIGN: u32 = 0x8;
const XKU_SGC: u32 = 0x10;
const XKU_OCSP_SIGN: u32 = 0x20;
const XKU_TIMESTAMP: u32 = 0x40;
const XKU_DVCS: u32 = 0x80;
const XKU_ANYEKU: u32 = 0x100;

const NS_SSL_CLIENT: u32 = 0x80;
const NS_SSL_SERVER: u32 = 0x40;
const NS_SMIME: u32 = 0x20;
const NS_SSL_CA: u32 = 0x04;
const NS_SMIME_CA: u32 = 0x02;
const NS_ANY_CA: u32 = 0x07;

struct Flags {
    bcons: bool,
    ca: bool,
    ku: Option<u32>,
    xku: Option<u32>,
    ns: Option<u32>,
    v1_root: bool,
    ku_critical: bool,
    xku_critical: bool,
}

impl Flags {
    fn ku_reject(&self, usage: u32) -> bool {
        self.ku.is_some_and(|k| k & usage == 0)
    }
    fn xku_reject(&self, usage: u32) -> bool {
        self.xku.is_some_and(|k| k & usage == 0)
    }
    fn ns_reject(&self, usage: u32) -> bool {
        self.ns.is_some_and(|k| k & usage == 0)
    }
    fn nscert(&self) -> u32 {
        self.ns.unwrap_or(0)
    }

    fn check_ca(&self) -> i32 {
        if self.ku_reject(KU_KEY_CERT_SIGN) {
            return 0;
        }
        if self.bcons {
            return i32::from(self.ca);
        }
        if self.v1_root {
            3
        } else if self.ku.is_some() {
            4
        } else if self.ns.is_some_and(|n| n & NS_ANY_CA != 0) {
            5
        } else {
            0
        }
    }

    fn check_ssl_ca(&self) -> i32 {
        let r = self.check_ca();
        if r == 0 {
            return 0;
        }
        i32::from(r != 5 || self.nscert() & NS_SSL_CA != 0)
    }

    fn ssl_client(&self, ca: bool) -> i32 {
        if self.xku_reject(XKU_SSL_CLIENT) {
            return 0;
        }
        if ca {
            return self.check_ssl_ca();
        }
        if self.ku_reject(KU_DIGITAL_SIGNATURE | KU_KEY_AGREEMENT) || self.ns_reject(NS_SSL_CLIENT) {
            return 0;
        }
        1
    }

    fn ssl_server(&self, ca: bool) -> i32 {
        if self.xku_reject(XKU_SSL_SERVER | XKU_SGC) {
            return 0;
        }
        if ca {
            return self.check_ssl_ca();
        }
        if self.ns_reject(NS_SSL_SERVER) || self.ku_reject(KU_DIGITAL_SIGNATURE | KU_KEY_ENCIPHERMENT | KU_KEY_AGREEMENT) {
            return 0;
        }
        1
    }

    fn ns_ssl_server(&self, ca: bool) -> i32 {
        let r = self.ssl_server(ca);
        if r == 0 || ca {
            return r;
        }
        if self.ku_reject(KU_KEY_ENCIPHERMENT) { 0 } else { r }
    }

    fn smime(&self, ca: bool) -> i32 {
        if self.xku_reject(XKU_SMIME) {
            return 0;
        }
        if ca {
            let r = self.check_ca();
            if r == 0 {
                return 0;
            }
            return if r != 5 || self.nscert() & NS_SMIME_CA != 0 { r } else { 0 };
        }
        if let Some(ns) = self.ns {
            if ns & NS_SMIME != 0 {
                return 1;
            }
            return if ns & NS_SSL_CLIENT != 0 { 2 } else { 0 };
        }
        1
    }

    fn smime_sign(&self, ca: bool) -> i32 {
        let r = self.smime(ca);
        if r == 0 || ca {
            return r;
        }
        if self.ku_reject(KU_DIGITAL_SIGNATURE | KU_NON_REPUDIATION) { 0 } else { r }
    }

    fn smime_encrypt(&self, ca: bool) -> i32 {
        let r = self.smime(ca);
        if r == 0 || ca {
            return r;
        }
        if self.ku_reject(KU_KEY_ENCIPHERMENT) { 0 } else { r }
    }

    fn crl_sign(&self, ca: bool) -> i32 {
        if ca {
            let r = self.check_ca();
            return if r == 2 { 0 } else { r };
        }
        i32::from(!self.ku_reject(KU_CRL_SIGN))
    }

    fn ocsp_helper(&self, ca: bool) -> i32 {
        if ca { self.check_ca() } else { 1 }
    }

    fn timestamp_sign(&self, ca: bool) -> i32 {
        if ca {
            return self.check_ca();
        }
        if let Some(ku) = self.ku
            && (ku & !(KU_NON_REPUDIATION | KU_DIGITAL_SIGNATURE) != 0 || ku & (KU_NON_REPUDIATION | KU_DIGITAL_SIGNATURE) == 0)
        {
            return 0;
        }
        if self.xku != Some(XKU_TIMESTAMP) || !self.xku_critical {
            return 0;
        }
        1
    }

    fn code_sign(&self, ca: bool) -> i32 {
        if ca {
            return self.check_ca();
        }
        let Some(ku) = self.ku else { return 0 };
        if ku & KU_DIGITAL_SIGNATURE == 0 || ku & (KU_KEY_CERT_SIGN | KU_CRL_SIGN) != 0 || !self.ku_critical {
            return 0;
        }
        let Some(xku) = self.xku else { return 0 };
        if xku & XKU_CODE_SIGN == 0 || xku & (XKU_ANYEKU | XKU_SSL_SERVER) != 0 {
            return 0;
        }
        1
    }
}

/// The bits of a BIT STRING extension (first byte, then the second's << 8).
fn bits(value: &[u8]) -> Option<u32> {
    let b = der::parse_one(value).filter(|b| b.tag == der::BIT_STRING)?;
    let d = b.body.get(1..)?;
    Some(u32::from(*d.first().unwrap_or(&0)) | (u32::from(*d.get(1).unwrap_or(&0)) << 8))
}

impl Cert {
    fn flags(&self) -> Flags {
        let mut f = Flags { bcons: false, ca: false, ku: None, xku: None, ns: None, v1_root: false, ku_critical: false, xku_critical: false };
        if let Some(e) = self.ext("2.5.29.19") {
            f.bcons = true;
            if let Some(s) = der::parse_one(&e.value) {
                f.ca = s.reader().optional(0x01).is_some_and(|b| b.body.first().is_some_and(|v| *v != 0));
            }
        }
        if let Some(e) = self.ext("2.5.29.15") {
            f.ku = Some(bits(&e.value).unwrap_or(0));
            f.ku_critical = e.critical;
        }
        if let Some(e) = self.ext("2.5.29.37") {
            let mut x = 0;
            if let Some(s) = der::parse_one(&e.value) {
                let mut r = s.reader();
                while let Some(o) = r.read() {
                    x |= match der::oid_text(o.body).as_str() {
                        "1.3.6.1.5.5.7.3.1" => XKU_SSL_SERVER,
                        "1.3.6.1.5.5.7.3.2" => XKU_SSL_CLIENT,
                        "1.3.6.1.5.5.7.3.4" => XKU_SMIME,
                        "1.3.6.1.5.5.7.3.3" => XKU_CODE_SIGN,
                        "1.3.6.1.4.1.311.10.3.3" | "2.16.840.1.113730.4.1" => XKU_SGC,
                        "1.3.6.1.5.5.7.3.9" => XKU_OCSP_SIGN,
                        "1.3.6.1.5.5.7.3.8" => XKU_TIMESTAMP,
                        "1.3.6.1.5.5.7.3.10" => XKU_DVCS,
                        "2.5.29.37.0" => XKU_ANYEKU,
                        _ => 0,
                    };
                }
            }
            f.xku = Some(x);
            f.xku_critical = e.critical;
        }
        if let Some(e) = self.ext("2.16.840.1.113730.1.1") {
            f.ns = Some(bits(&e.value).unwrap_or(0) & 0xff);
        }
        if self.version == 0 && self.subject.same(&self.issuer) {
            let key_kind = self.public_key().map(|k| k.keytype());
            let sig_kind = sig_alg(&self.sig_oid).map(|(_, k)| k);
            f.v1_root = matches!((key_kind, sig_kind), (Some(0), Some("rsa")) | (Some(3), Some("ec")) | (Some(5), Some("ed25519")));
        }
        f
    }

    /// (purpose id, short name, long name, as a leaf, as a CA).
    pub fn purposes(&self) -> Vec<(i64, &'static str, &'static str, bool, bool)> {
        let f = self.flags();
        type Check = fn(&Flags, bool) -> i32;
        let list: [(i64, &str, &str, Check); 10] = [
            (1, "sslclient", "SSL client", Flags::ssl_client),
            (2, "sslserver", "SSL server", Flags::ssl_server),
            (3, "nssslserver", "Netscape SSL server", Flags::ns_ssl_server),
            (4, "smimesign", "S/MIME signing", Flags::smime_sign),
            (5, "smimeencrypt", "S/MIME encryption", Flags::smime_encrypt),
            (6, "crlsign", "CRL signing", Flags::crl_sign),
            (7, "any", "Any Purpose", |_, _| 1),
            (8, "ocsphelper", "OCSP helper", Flags::ocsp_helper),
            (9, "timestampsign", "Time Stamp signing", Flags::timestamp_sign),
            (10, "codesign", "Code signing", Flags::code_sign),
        ];
        list.iter().map(|(id, sn, ln, check)| (*id, *sn, *ln, check(&f, false) != 0, check(&f, true) != 0)).collect()
    }

    /// X509_check_purpose(cert, id, 0) for openssl_x509_checkpurpose.
    pub fn check_purpose(&self, id: i64) -> Option<bool> {
        self.purposes().into_iter().find(|p| p.0 == id).map(|p| p.3)
    }
}

// ---- extensions as X509V3_EXT_print prints them ----

/// GENERAL_NAME_print, as `ldap` decides whether DirName is printed one-line (true) or as RFC 2253-style.
fn general_name(t: &Tlv, print: bool) -> (String, Vec<u8>) {
    match t.tag {
        0x81 => ("email".into(), t.body.to_vec()),
        0x82 => ("DNS".into(), t.body.to_vec()),
        0x86 => ("URI".into(), t.body.to_vec()),
        0xa4 => {
            let name = der::parse_one(t.body).and_then(|n| parse_name(&n));
            let shown = name.map(|n| if print { n.print_oneline() } else { n.oneline() }).unwrap_or_default();
            ("DirName".into(), shown)
        }
        0x87 => ("IP Address".into(), ip(t.body).into_bytes()),
        0x88 => ("Registered ID".into(), oids::ln_or_dotted(&der::oid_text(t.body)).into_bytes()),
        0xa0 => ("othername".into(), b"<unsupported>".to_vec()),
        0xa3 => ("X400Name".into(), b"<unsupported>".to_vec()),
        0xa5 => ("EdiPartyName".into(), b"<unsupported>".to_vec()),
        _ => ("othername".into(), b"<unsupported>".to_vec()),
    }
}

fn ip(b: &[u8]) -> String {
    match b.len() {
        4 => format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]),
        16 => b.chunks(2).map(|c| format!("{:X}", u16::from_be_bytes([c[0], c[1]]))).collect::<Vec<_>>().join(":"),
        _ => "<invalid>".into(),
    }
}

fn name_value(n: &str, v: &[u8]) -> Vec<u8> {
    let mut out = n.as_bytes().to_vec();
    out.push(b':');
    out.extend_from_slice(v);
    out
}

fn general_names(data: &[u8], print: bool) -> Option<Vec<Vec<u8>>> {
    let s = der::parse_one(data).filter(|s| s.tag == der::SEQUENCE)?;
    let mut r = s.reader();
    let mut out = Vec::new();
    while let Some(t) = r.read() {
        let (n, v) = general_name(&t, print);
        out.push(name_value(&n, &v));
    }
    Some(out)
}

/// The subjectAltName as PHP's openssl_x509v3_subjectAltName writes it.
pub fn subject_alt_name(value: &[u8]) -> Option<Vec<u8>> {
    Some(general_names(value, true)?.join(&b", "[..]))
}

/// X509V3_EXT_print(ext, 0, 0): None when OpenSSL has no printer for it
/// (PHP then shows the raw value).
pub fn print_ext(e: &Ext) -> Option<Vec<u8>> {
    let v = &e.value;
    let join = |items: Vec<Vec<u8>>, sep: &[u8]| items.join(sep);
    match e.oid.as_str() {
        "2.5.29.19" => {
            let s = der::parse_one(v)?;
            let mut r = s.reader();
            let ca = r.optional(0x01).is_some_and(|b| b.body.first().is_some_and(|x| *x != 0));
            let mut out = if ca { b"CA:TRUE".to_vec() } else { b"CA:FALSE".to_vec() };
            if let Some(p) = r.optional(der::INTEGER) {
                let n = p.body.iter().fold(0i64, |a, b| (a << 8) | i64::from(*b));
                out.extend_from_slice(format!(", pathlen:{n}").as_bytes());
            }
            Some(out)
        }
        "2.5.29.15" => {
            let names = ["Digital Signature", "Non Repudiation", "Key Encipherment", "Data Encipherment", "Key Agreement", "Certificate Sign", "CRL Sign", "Encipher Only", "Decipher Only"];
            let b = der::parse_one(v).filter(|b| b.tag == der::BIT_STRING)?;
            let data = b.body.get(1..)?;
            let set: Vec<Vec<u8>> = names.iter().enumerate().filter(|(i, _)| data.get(i / 8).is_some_and(|byte| byte & (0x80 >> (i % 8)) != 0)).map(|(_, n)| n.as_bytes().to_vec()).collect();
            Some(join(set, b", "))
        }
        "2.16.840.1.113730.1.1" => {
            let names = ["SSL Client", "SSL Server", "S/MIME", "Object Signing", "Unused", "SSL CA", "S/MIME CA", "Object Signing CA"];
            let b = der::parse_one(v).filter(|b| b.tag == der::BIT_STRING)?;
            let data = b.body.get(1..)?;
            let set: Vec<Vec<u8>> = names.iter().enumerate().filter(|(i, _)| data.first().is_some_and(|byte| byte & (0x80 >> i) != 0)).map(|(_, n)| n.as_bytes().to_vec()).collect();
            Some(join(set, b", "))
        }
        "2.5.29.37" => {
            let s = der::parse_one(v)?;
            let mut r = s.reader();
            let mut out = Vec::new();
            while let Some(o) = r.read() {
                out.push(oids::ln_or_dotted(&der::oid_text(o.body)).into_bytes());
            }
            Some(join(out, b", "))
        }
        "2.5.29.14" => {
            let o = der::parse_one(v).filter(|o| o.tag == der::OCTET_STRING)?;
            Some(der::hex_upper(o.body, ":").into_bytes())
        }
        "2.5.29.35" => {
            let s = der::parse_one(v)?;
            let mut r = s.reader();
            let mut parts: Vec<(Option<&str>, Vec<u8>)> = Vec::new();
            let mut keyid = None;
            let mut rest = Vec::new();
            while let Some(t) = r.read() {
                match t.tag {
                    0x80 => keyid = Some(der::hex_upper(t.body, ":").into_bytes()),
                    0xa1 => {
                        let mut g = t.reader();
                        while let Some(n) = g.read() {
                            let (name, value) = general_name(&n, false);
                            rest.push(name_value(&name, &value));
                        }
                    }
                    0x82 => {
                        // the number's bytes: no sign byte, but "00" for zero
                        let n = crate::pkey::minimal(t.body);
                        let n = if n.is_empty() { vec![0] } else { n };
                        rest.push(name_value("serial", der::hex_upper(&n, ":").as_bytes()));
                    }
                    _ => {}
                }
            }
            if let Some(k) = keyid {
                parts.push((if rest.is_empty() { None } else { Some("keyid") }, k));
            }
            let mut lines: Vec<Vec<u8>> = parts.into_iter().map(|(n, v)| match n {
                Some(n) => name_value(n, &v),
                None => v,
            }).collect();
            lines.extend(rest);
            Some(join(lines, b"\n"))
        }
        "2.5.29.18" => Some(join(general_names(v, false)?, b", ")),
        "1.3.6.1.5.5.7.1.1" | "1.3.6.1.5.5.7.1.11" => {
            let s = der::parse_one(v)?;
            let mut r = s.reader();
            let mut lines = Vec::new();
            while let Some(ad) = r.read() {
                let mut a = ad.reader();
                let method = oids::ln_or_dotted(&der::oid_text(a.expect(der::OID)?.body));
                let loc = a.read()?;
                let (n, val) = general_name(&loc, false);
                lines.push(name_value(&format!("{method} - {n}"), &val));
            }
            Some(join(lines, b"\n"))
        }
        "2.5.29.31" | "2.5.29.46" => {
            let s = der::parse_one(v)?;
            let mut r = s.reader();
            let mut out = Vec::new();
            let mut first = true;
            while let Some(dp) = r.read() {
                if !first {
                    out.push(b'\n');
                }
                first = false;
                let mut d = dp.reader();
                while let Some(t) = d.read() {
                    match t.tag {
                        0xa0 => {
                            let inner = der::parse_one(t.body)?;
                            if inner.tag == 0xa0 {
                                out.extend_from_slice(b"Full Name:\n");
                                let mut g = inner.reader();
                                let mut names = Vec::new();
                                while let Some(n) = g.read() {
                                    let (name, value) = general_name(&n, true);
                                    let mut line = b"  ".to_vec();
                                    line.extend(name_value(&name, &value));
                                    names.push(line);
                                }
                                out.extend(join(names, b"\n"));
                                out.push(b'\n');
                            } else {
                                out.extend_from_slice(b"Relative Name:\n");
                            }
                        }
                        0x81 => {
                            out.extend_from_slice(b"Reasons:");
                            out.push(b'\n');
                        }
                        0xa2 => {
                            out.extend_from_slice(b"CRL Issuer:\n");
                            let mut g = t.reader();
                            let mut names = Vec::new();
                            while let Some(n) = g.read() {
                                let (name, value) = general_name(&n, true);
                                let mut line = b"  ".to_vec();
                                line.extend(name_value(&name, &value));
                                names.push(line);
                            }
                            out.extend(join(names, b"\n"));
                        }
                        _ => {}
                    }
                }
            }
            Some(out)
        }
        "2.5.29.32" => {
            let s = der::parse_one(v)?;
            let mut r = s.reader();
            let mut out = Vec::new();
            let mut first = true;
            while let Some(pi) = r.read() {
                if !first {
                    out.push(b'\n');
                }
                first = false;
                let mut p = pi.reader();
                let oid = der::oid_text(p.expect(der::OID)?.body);
                out.extend_from_slice(b"Policy: ");
                out.extend_from_slice(oids::ln_or_dotted(&oid).as_bytes());
                if let Some(quals) = p.read() {
                    out.push(b'\n');
                    let mut q = quals.reader();
                    let mut items = Vec::new();
                    while let Some(qi) = q.read() {
                        let mut qr = qi.reader();
                        let qid = der::oid_text(qr.expect(der::OID)?.body);
                        let mut line = Vec::new();
                        match qid.as_str() {
                            "1.3.6.1.5.5.7.2.1" => {
                                let cps = qr.read()?;
                                line.extend_from_slice(b"  CPS: ");
                                line.extend_from_slice(until_nul(cps.body));
                            }
                            "1.3.6.1.5.5.7.2.2" => {
                                line.extend_from_slice(b"  User Notice:");
                                if let Some(un) = qr.read() {
                                    let mut u = un.reader();
                                    while let Some(part) = u.read() {
                                        if part.tag == der::SEQUENCE {
                                            let mut nr = part.reader();
                                            let org = nr.read()?;
                                            line.extend_from_slice(b"\n    Organization: ");
                                            line.extend_from_slice(until_nul(org.body));
                                            if let Some(nums) = nr.read() {
                                                let mut ns = nums.reader();
                                                let mut list = Vec::new();
                                                while let Some(n) = ns.read() {
                                                    list.push(n.body.iter().fold(0i64, |a, b| (a << 8) | i64::from(*b)).to_string());
                                                }
                                                let plural = if list.len() > 1 { "s" } else { "" };
                                                line.extend_from_slice(format!("\n    Number{plural}: {}", list.join(", ")).as_bytes());
                                            }
                                        } else {
                                            line.extend_from_slice(b"\n    Explicit Text: ");
                                            line.extend_from_slice(until_nul(part.body));
                                        }
                                    }
                                }
                            }
                            _ => {
                                line.extend_from_slice(b"  Unknown Qualifier: ");
                                line.extend_from_slice(oids::ln_or_dotted(&qid).as_bytes());
                            }
                        }
                        items.push(line);
                    }
                    out.extend(join(items, b"\n"));
                }
            }
            Some(out)
        }
        "2.16.840.1.113730.1.13" | "2.16.840.1.113730.1.2" | "2.16.840.1.113730.1.3" | "2.16.840.1.113730.1.4" | "2.16.840.1.113730.1.7" | "2.16.840.1.113730.1.8" | "2.16.840.1.113730.1.12" => {
            let s = der::parse_one(v)?;
            Some(s.body.to_vec())
        }
        _ => None,
    }
}

fn until_nul(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|c| *c == 0).unwrap_or(b.len())]
}

/// An extension's name in openssl_x509_parse(): its short name, or the dotted OID.
pub fn ext_name(oid: &str) -> String {
    oids::by_oid(oid).map_or_else(|| oid.to_string(), |o| o.sn.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_t_is_utc() {
        assert_eq!(time_t(der::UTC_TIME, b"080630102843Z"), Ok(1214821723));
        assert_eq!(time_t(der::GENERALIZED_TIME, b"20380119031408Z"), Ok(2147483648));
    }

    #[test]
    fn serials_print_as_openssl_does() {
        assert_eq!(serial_strings(&[0x00, 0xae, 0xc5, 0x56, 0xcc, 0x72, 0x37, 0x50, 0xa2]), ("12593567369101004962".into(), "AEC556CC723750A2".into()));
        assert_eq!(serial_strings(&[0x0a]), ("10".into(), "0A".into()));
        assert_eq!(serial_strings(&[0x00]), ("0".into(), "0".into()));
    }
}
