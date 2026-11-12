//! CURLOPT_POSTFIELDS as an array: the multipart/form-data body
//! build_mime_structure_from_hash() makes (CURLFile and CURLStringFile
//! parts, arrays of values as repeated fields).

use crate::exec::CurlErr;
use crate::{bytes_of, lossy};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::value::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct Form {
    pub boundary: String,
    pub body: Vec<u8>,
}

/// libcurl quotes names and file names in a header value.
fn quoted(s: &[u8]) -> Vec<u8> {
    let mut out = vec![b'"'];
    for &b in s {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\r' => out.extend_from_slice(b"%0D"),
            b'\n' => out.extend_from_slice(b"%0A"),
            _ => out.push(b),
        }
    }
    out.push(b'"');
    out
}

fn boundary() -> String {
    let mut x = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0x9e37_79b9, |d| d.as_nanos() as u64) | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    // libcurl: 24 dashes and 22 random alphanumerics.
    const ALNUM: &[u8; 62] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut s = String::from("------------------------");
    for _ in 0..22 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.push(ALNUM[(x % 62) as usize] as char);
    }
    s
}

fn part(body: &mut Vec<u8>, boundary: &str, name: &[u8], filename: Option<&[u8]>, mime: Option<&[u8]>, data: &[u8]) {
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=").as_bytes());
    body.extend_from_slice(&quoted(name));
    if let Some(f) = filename {
        body.extend_from_slice(b"; filename=");
        body.extend_from_slice(&quoted(f));
    }
    body.extend_from_slice(b"\r\n");
    if let Some(m) = mime {
        body.extend_from_slice(b"Content-Type: ");
        body.extend_from_slice(m);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(data);
    body.extend_from_slice(b"\r\n");
}

fn prop(o: &p8_runtime::object::Object, name: &[u8], class: &[u8]) -> Value {
    o.class.read(o, name, class)
}

/// The multipart body for `fields`; the CurlErr when a file cannot be read.
pub(crate) fn build(ctx: &mut dyn Ctx, fields: &PhpArray) -> Result<Result<Form, CurlErr>, Throw> {
    let boundary = boundary();
    let mut body = Vec::new();
    for (key, value) in fields.iter() {
        let name = match &key {
            Key::Int(i) => i.to_string().into_bytes(),
            Key::Str(s) => s.as_bytes().to_vec(),
        };
        let value = value.deref();
        if let Some(o) = value.as_object()
            && o.class.is_a(b"curlfile")
        {
            let Ok(path) = prop(o, b"name", b"CURLFile").into_str() else {
                let func = ctx.builtin_name();
                ctx.warn(&format!("{func}(): Invalid filename for key {}", lossy(&name)));
                continue;
            };
            let mime = match prop(o, b"mime", b"CURLFile").as_str() {
                Some(m) if !m.is_empty() => m.as_bytes().to_vec(),
                _ => b"application/octet-stream".to_vec(),
            };
            let postname = match prop(o, b"postname", b"CURLFile").as_str() {
                Some(m) if !m.is_empty() => m.as_bytes().to_vec(),
                _ => path.as_bytes().to_vec(),
            };
            let Ok(data) = std::fs::read(p8_runtime::cwd::resolve(ctx, lossy(path.as_bytes()))) else {
                return Ok(Err(CurlErr { code: 26, message: "Failed to open/read local data from file/application".into() }));
            };
            part(&mut body, &boundary, &name, Some(&postname), Some(&mime), &data);
        } else if let Some(o) = value.as_object()
            && o.class.is_a(b"curlstringfile")
        {
            let data = bytes_of(&prop(o, b"data", b"CURLStringFile"));
            let postname = bytes_of(&prop(o, b"postname", b"CURLStringFile"));
            let mime = bytes_of(&prop(o, b"mime", b"CURLStringFile"));
            part(&mut body, &boundary, &name, Some(&postname), Some(&mime), &data);
        } else if let Some(items) = value.as_array() {
            for item in items.values() {
                part(&mut body, &boundary, &name, None, None, &bytes_of(&item.deref()));
            }
        } else {
            part(&mut body, &boundary, &name, None, None, &bytes_of(&value));
        }
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(Ok(Form { boundary, body }))
}
