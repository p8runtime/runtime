//! DER and PEM: the small reader and writer the certificate and key code
//! needs (single-byte tags, definite lengths), base64 as OpenSSL's PEM
//! code reads and writes it, and object identifiers as dotted text.

pub const INTEGER: u8 = 0x02;
pub const BIT_STRING: u8 = 0x03;
pub const OCTET_STRING: u8 = 0x04;
pub const NULL: u8 = 0x05;
pub const OID: u8 = 0x06;
pub const UTF8_STRING: u8 = 0x0c;
pub const PRINTABLE_STRING: u8 = 0x13;
pub const T61_STRING: u8 = 0x14;
pub const IA5_STRING: u8 = 0x16;
pub const UTC_TIME: u8 = 0x17;
pub const GENERALIZED_TIME: u8 = 0x18;
pub const VISIBLE_STRING: u8 = 0x1a;
pub const UNIVERSAL_STRING: u8 = 0x1c;
pub const BMP_STRING: u8 = 0x1e;
pub const SEQUENCE: u8 = 0x30;
pub const SET: u8 = 0x31;

/// One element: its tag, its contents, and the whole encoding.
#[derive(Clone, Copy, Debug)]
pub struct Tlv<'a> {
    pub tag: u8,
    pub body: &'a [u8],
    pub raw: &'a [u8],
}

impl<'a> Tlv<'a> {
    /// A reader over the contents (of a constructed element).
    pub fn reader(&self) -> Reader<'a> {
        Reader::new(self.body)
    }
}

/// Reads elements one after another.
#[derive(Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub fn peek_tag(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    pub fn read(&mut self) -> Option<Tlv<'a>> {
        let start = self.pos;
        let tag = *self.data.get(self.pos)?;
        if tag & 0x1f == 0x1f {
            return None; // multi-byte tags are not used here
        }
        let first = *self.data.get(self.pos + 1)?;
        let mut at = self.pos + 2;
        let len = if first < 0x80 {
            first as usize
        } else {
            let n = (first & 0x7f) as usize;
            if n == 0 || n > 4 {
                return None;
            }
            let mut len = 0usize;
            for _ in 0..n {
                len = (len << 8) | *self.data.get(at)? as usize;
                at += 1;
            }
            len
        };
        let end = at.checked_add(len)?;
        if end > self.data.len() {
            return None;
        }
        self.pos = end;
        Some(Tlv { tag, body: &self.data[at..end], raw: &self.data[start..end] })
    }

    /// The next element, which must have `tag`.
    pub fn expect(&mut self, tag: u8) -> Option<Tlv<'a>> {
        if self.peek_tag()? != tag {
            return None;
        }
        self.read()
    }

    /// The next element if it has `tag`.
    pub fn optional(&mut self, tag: u8) -> Option<Tlv<'a>> {
        if self.peek_tag() == Some(tag) { self.read() } else { None }
    }
}

/// Parse exactly one element spanning `data`.
pub fn parse_one(data: &[u8]) -> Option<Tlv<'_>> {
    let mut r = Reader::new(data);
    let t = r.read()?;
    r.is_empty().then_some(t)
}

// ---- writing ----

pub fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = body.len();
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes = n.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(body);
    out
}

pub fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(SEQUENCE, &parts.concat())
}

/// An unsigned big-endian number as an INTEGER.
pub fn uint(bytes: &[u8]) -> Vec<u8> {
    let skip = bytes.iter().take_while(|b| **b == 0).count();
    let mut body = bytes[skip..].to_vec();
    if body.is_empty() || body[0] & 0x80 != 0 {
        body.insert(0, 0);
    }
    tlv(INTEGER, &body)
}

pub fn small_int(i: u64) -> Vec<u8> {
    uint(&i.to_be_bytes())
}

pub fn bit_string(bytes: &[u8]) -> Vec<u8> {
    let mut body = vec![0u8];
    body.extend_from_slice(bytes);
    tlv(BIT_STRING, &body)
}

pub fn octet_string(bytes: &[u8]) -> Vec<u8> {
    tlv(OCTET_STRING, bytes)
}

pub fn null() -> Vec<u8> {
    vec![NULL, 0]
}

// ---- object identifiers ----

pub fn oid(dotted: &str) -> Vec<u8> {
    tlv(OID, &oid_body(dotted))
}

pub fn oid_body(dotted: &str) -> Vec<u8> {
    let arcs: Vec<u64> = dotted.split('.').map(|a| a.parse().unwrap_or(0)).collect();
    let mut out = Vec::new();
    if arcs.len() < 2 {
        return out;
    }
    let mut push = |mut v: u64| {
        let mut tmp = vec![(v & 0x7f) as u8];
        v >>= 7;
        while v > 0 {
            tmp.push(0x80 | (v & 0x7f) as u8);
            v >>= 7;
        }
        tmp.reverse();
        out.extend(tmp);
    };
    push(arcs[0] * 40 + arcs[1]);
    for a in &arcs[2..] {
        push(*a);
    }
    out
}

pub fn oid_text(body: &[u8]) -> String {
    let mut arcs: Vec<u128> = Vec::new();
    let mut v: u128 = 0;
    for b in body {
        v = (v << 7) | u128::from(b & 0x7f);
        if b & 0x80 == 0 {
            arcs.push(v);
            v = 0;
        }
    }
    let mut out = String::new();
    if let Some(first) = arcs.first() {
        let (a, b) = if *first < 40 {
            (0, *first)
        } else if *first < 80 {
            (1, first - 40)
        } else {
            (2, first - 80)
        };
        out.push_str(&format!("{a}.{b}"));
    }
    for a in arcs.iter().skip(1) {
        out.push_str(&format!(".{a}"));
    }
    out
}

// ---- base64 ----

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Base64 with whitespace ignored; None on any other stray character.
pub fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut pad = 0;
    for &c in text {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                pad += 1;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        if pad > 0 {
            return None;
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// ---- PEM ----

/// One PEM block: its label ("CERTIFICATE"), its headers (Proc-Type,
/// DEK-Info), its decoded contents.
pub struct Pem {
    pub label: String,
    pub headers: Vec<(String, String)>,
    pub der: Vec<u8>,
}

/// Every well-formed PEM block in `text`, in order.
pub fn pem_blocks(text: &[u8]) -> Vec<Pem> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = find(rest, b"-----BEGIN ") {
        let after = &rest[at + 11..];
        let Some(dash) = find(after, b"-----") else { break };
        let label = String::from_utf8_lossy(&after[..dash]).into_owned();
        let body_start = &after[dash + 5..];
        let end_marker = format!("-----END {label}-----");
        let Some(end) = find(body_start, end_marker.as_bytes()) else {
            rest = body_start;
            continue;
        };
        let body = &body_start[..end];
        rest = &body_start[end + end_marker.len()..];
        if label.contains('\n') {
            continue;
        }
        let mut headers = Vec::new();
        let mut b64 = Vec::new();
        let lines: Vec<&[u8]> = body.split(|c| *c == b'\n').collect();
        let mut in_headers = true;
        for line in lines {
            let line = trim(line);
            if in_headers && line.contains(&b':') {
                let s = String::from_utf8_lossy(line);
                if let Some((k, v)) = s.split_once(':') {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
                continue;
            }
            if !line.is_empty() {
                in_headers = false;
            }
            b64.extend_from_slice(line);
        }
        if let Some(der) = base64_decode(&b64) {
            out.push(Pem { label, headers, der });
        }
    }
    out
}

/// The first block whose label is one of `labels`.
pub fn pem_find(text: &[u8], labels: &[&str]) -> Option<Pem> {
    pem_blocks(text).into_iter().find(|p| labels.contains(&p.label.as_str()))
}

pub fn pem_encode(label: &str, der: &[u8]) -> String {
    pem_encode_with(label, &[], der)
}

pub fn pem_encode_with(label: &str, headers: &[(&str, String)], der: &[u8]) -> String {
    let mut out = format!("-----BEGIN {label}-----\n");
    if !headers.is_empty() {
        for (k, v) in headers {
            out.push_str(&format!("{k}: {v}\n"));
        }
        out.push('\n');
    }
    let b = base64_encode(der);
    for chunk in b.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn trim(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(s.len());
    let end = s.iter().rposition(|c| !c.is_ascii_whitespace()).map_or(start, |e| e + 1);
    &s[start..end.max(start)]
}

pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

pub fn hex_upper(bytes: &[u8], sep: &str) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(sep)
}

pub fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oids_round_trip() {
        for o in ["1.2.840.113549.1.1.11", "2.5.4.3", "1.3.6.1.4.1.311.60.2.1.3", "2.999.3"] {
            assert_eq!(oid_text(&oid_body(o)), o);
        }
    }

    #[test]
    fn base64_round_trips() {
        for s in [&b""[..], b"a", b"ab", b"abc", b"abcd\x00\xff"] {
            assert_eq!(base64_decode(base64_encode(s).as_bytes()).unwrap(), s);
        }
    }

    #[test]
    fn long_lengths() {
        let body = vec![7u8; 300];
        let enc = tlv(OCTET_STRING, &body);
        let t = parse_one(&enc).unwrap();
        assert_eq!(t.body, &body[..]);
    }
}
