//! libmagic's file_buffer(): the order in which the checks run and how their
//! output is assembled (funcs.c, ascmagic.c).

use crate::magic::{Env, F_APPLE, F_CONTINUE, F_EXTENSION, F_MIME_ENC, F_MIME_TYPE, Failure, Rules, softmagic};
use crate::text::{self, Encoding};

const ENCODING_MAX: usize = 64 * 1024;
const MAXLINELEN: usize = 300;

fn trim_nuls(b: &[u8]) -> usize {
    let mut n = b.len();
    while n > 1 && b[n - 1] == 0 {
        n -= 1;
    }
    n
}

/// file_default
fn file_default(flags: i64, nb: usize, out: &mut Vec<u8>) -> bool {
    if flags & (F_MIME_TYPE | F_MIME_ENC) != 0 {
        if flags & F_MIME_TYPE != 0 {
            out.extend(if nb != 0 { &b"application/octet-stream"[..] } else { b"application/x-empty" });
        }
        return true;
    }
    if flags & F_APPLE != 0 {
        out.extend(b"UNKNUNKN");
        return true;
    }
    if flags & F_EXTENSION != 0 {
        out.extend(b"???");
        return true;
    }
    false
}

const SEPARATOR: &[u8] = b"\n- ";

fn finish(flags: i64, mut out: Vec<u8>, code_mime: &str) -> Vec<u8> {
    // trim_separator
    if out.ends_with(SEPARATOR) {
        out.truncate(out.len() - SEPARATOR.len());
    }
    if flags & F_MIME_ENC != 0 {
        if flags & F_MIME_TYPE != 0 {
            out.extend(b"; charset=");
        }
        out.extend(code_mime.bytes());
    }
    out
}

/// checkdone: without FILEINFO_CONTINUE the first answer is the answer.
fn checkdone(flags: i64, out: &mut Vec<u8>) -> bool {
    if flags & F_CONTINUE == 0 {
        return true;
    }
    out.extend(SEPARATOR);
    false
}

/// file_replace: `pat$` (multiline, so also before every newline) becomes `to`;
/// the number of replacements.
fn replace_eol(out: &mut Vec<u8>, pat: &[u8], to: &[u8]) -> usize {
    let mut res = Vec::with_capacity(out.len());
    let mut n = 0;
    let mut i = 0;
    while i < out.len() {
        if out[i..].starts_with(pat) && out.get(i + pat.len()).is_none_or(|&c| c == b'\n') {
            res.extend(to);
            i += pat.len();
            n += 1;
        } else {
            res.push(out[i]);
            i += 1;
        }
    }
    *out = res;
    n
}

pub fn identify(rules: &Rules, buf: &[u8], flags: i64, env: Env) -> Result<Vec<u8>, String> {
    let nb = buf.len();
    let mime = flags & (F_MIME_TYPE | F_MIME_ENC) != 0;
    let mut out: Vec<u8> = Vec::new();
    let mut def = "data";
    let mut code_mime = "binary";
    if nb == 0 || nb == 1 {
        def = if nb == 0 { "empty" } else { "very short file (no magic)" };
        return Ok(simple(flags, nb, out, def, code_mime));
    }
    let enc = text::file_encoding(&buf[..nb.min(ENCODING_MAX)]);
    code_mime = enc.mime;
    let looks_text = enc.looks_text;
    let no_desc = flags & (F_APPLE | F_EXTENSION) != 0;

    let mut matched = false;
    // tar
    if !no_desc {
        let tar = text::is_tar(buf);
        if (1..=3).contains(&tar) {
            if flags & (F_MIME_TYPE | F_MIME_ENC) != F_MIME_ENC {
                out.extend(if mime { &b"application/x-tar"[..] } else { [&b"tar archive"[..], b"POSIX tar archive", b"POSIX tar archive (GNU)"][tar as usize - 1] });
            }
            matched = true;
            if checkdone(flags, &mut out) {
                return Ok(finish(flags, out, code_mime));
            }
        }
        // JSON
        let jt = text::is_json(buf);
        if jt != 0 {
            if flags & (F_MIME_TYPE | F_MIME_ENC) != F_MIME_ENC {
                if mime {
                    out.extend(format!("application/{}", if jt == 1 { "json" } else { "x-ndjson" }).bytes());
                } else {
                    out.extend(format!("{}JSON text data", if jt == 1 { "" } else { "New Line Delimited " }).bytes());
                }
            }
            matched = true;
            if checkdone(flags, &mut out) {
                return Ok(finish(flags, out, code_mime));
            }
        }
        // CSV
        if looks_text && text::is_csv(buf) {
            if flags & (F_MIME_TYPE | F_MIME_ENC) != F_MIME_ENC {
                if mime {
                    out.extend(b"text/csv");
                } else {
                    out.extend(format!("CSV {} text", enc.code).bytes());
                }
            }
            matched = true;
            if checkdone(flags, &mut out) {
                return Ok(finish(flags, out, code_mime));
            }
        }
    }
    // SIMH tape
    if !no_desc && text::is_simh(buf) {
        if flags & (F_MIME_TYPE | F_MIME_ENC) != F_MIME_ENC {
            out.extend(if mime { &b"application/SIMH-tape-data"[..] } else { b"SIMH tape data" });
        }
        matched = true;
        if checkdone(flags, &mut out) {
            return Ok(finish(flags, out, code_mime));
        }
    }
    // soft magic, binary tests
    match softmagic(rules, buf, true, looks_text, flags, &mut out, env) {
        Ok(true) => {
            matched = true;
            if checkdone(flags, &mut out) {
                return Ok(finish(flags, out, code_mime));
            }
        }
        Ok(false) => {}
        Err(Failure::Error(e)) => return Err(e),
    }
    // text properties
    match ascmagic(rules, buf, looks_text, flags, &mut out, env) {
        Ok(true) => return Ok(finish(flags, out, code_mime)),
        Ok(false) => {}
        Err(e) => return Err(e),
    }
    if matched {
        return Ok(finish(flags, out, code_mime));
    }
    Ok(simple(flags, nb, out, def, code_mime))
}

fn simple(flags: i64, nb: usize, mut out: Vec<u8>, def: &str, code_mime: &str) -> Vec<u8> {
    if !file_default(flags, nb, &mut out) {
        out.extend(def.bytes());
    }
    finish(flags, out, code_mime)
}

fn ascmagic(rules: &Rules, buf: &[u8], looks_text: bool, flags: i64, out: &mut Vec<u8>, env: Env) -> Result<bool, String> {
    let mut flen = trim_nuls(buf);
    if (flen & 1) == 1 && buf.len() & 1 == 0 {
        flen += 1;
    }
    let bb = &buf[..flen];
    let enc: Encoding = text::file_encoding(&bb[..bb.len().min(ENCODING_MAX)]);
    if !enc.looks_text {
        return Ok(false);
    }
    with_encoding(rules, bb, &enc, looks_text, flags, out, env)
}

fn with_encoding(rules: &Rules, buf: &[u8], enc: &Encoding, text_flag: bool, flags: i64, outp: &mut Vec<u8>, env: Env) -> Result<bool, String> {
    let nbytes = trim_nuls(buf);
    let mime = flags & (F_MIME_TYPE | F_MIME_ENC) != 0;
    let mut need_separator = false;
    if nbytes <= 1 {
        return Ok(false);
    }
    let ulen = enc.ubuf.len();
    if ulen > 0 {
        let Some(utf8) = text::encode_utf8(&enc.ubuf) else { return Ok(false) };
        match softmagic(rules, &utf8, false, text_flag, flags, outp, env) {
            Ok(m) => {
                need_separator = m;
                if flags & (F_APPLE | F_EXTENSION) != 0 {
                    return Ok(m);
                }
            }
            // an error in the text rules is not reported: the text is described without them
            Err(Failure::Error(_)) => {}
        }
    }
    if flags & (F_APPLE | F_EXTENSION) != 0 {
        return Ok(false);
    }
    // line terminators and the rest
    let (mut n_crlf, mut n_lf, mut n_cr, mut n_nel) = (0usize, 0usize, 0usize, 0usize);
    let mut seen_cr = false;
    let (mut has_escapes, mut has_backspace) = (false, false);
    let mut last_line_end: usize = usize::MAX;
    let mut has_long_lines = 0usize;
    for (i, &c) in enc.ubuf.iter().enumerate() {
        if c == u32::from(b'\n') {
            if seen_cr {
                n_crlf += 1;
            } else {
                n_lf += 1;
            }
            last_line_end = i;
        } else if seen_cr {
            n_cr += 1;
        }
        seen_cr = c == u32::from(b'\r');
        if seen_cr {
            last_line_end = i;
        }
        if c == 0x85 {
            n_nel += 1;
            last_line_end = i;
        }
        if i > last_line_end.wrapping_add(MAXLINELEN) {
            let ll = i.wrapping_sub(last_line_end);
            if ll > has_long_lines {
                has_long_lines = ll;
            }
        }
        if c == 0x1b {
            has_escapes = true;
        }
        if c == 8 {
            has_backspace = true;
        }
    }
    if enc.kind == "binary" {
        return Ok(false);
    }
    let out = outp;
    let len = out.len();
    if mime {
        if flags & F_MIME_TYPE != 0 {
            if len != 0 {
                // Softmagic printed something: we are either done, or we need a separator
                if flags & F_CONTINUE == 0 {
                    return Ok(true);
                }
                if need_separator {
                    out.extend(SEPARATOR);
                }
            }
            out.extend(b"text/plain");
        }
        return Ok(true);
    }
    let mut executable = false;
    if len != 0 && replace_eol(out, b" text", b", ") == 0 {
        if replace_eol(out, b" text executable", b", ") != 0 {
            executable = true;
        } else {
            out.extend(b", ");
        }
    }
    out.extend(enc.code.bytes());
    out.extend(b" text");
    if executable {
        out.extend(b" executable");
    }
    if has_long_lines != 0 {
        out.extend(format!(", with very long lines ({has_long_lines})").bytes());
    }
    if (n_crlf == 0 && n_cr == 0 && n_nel == 0 && n_lf == 0) || (n_crlf != 0 || n_cr != 0 || n_nel != 0) {
        out.extend(b", with");
        if n_crlf == 0 && n_cr == 0 && n_nel == 0 && n_lf == 0 {
            out.extend(b" no");
        } else {
            if n_crlf != 0 {
                out.extend(b" CRLF");
                if n_cr != 0 || n_lf != 0 || n_nel != 0 {
                    out.push(b',');
                }
            }
            if n_cr != 0 {
                out.extend(b" CR");
                if n_lf != 0 || n_nel != 0 {
                    out.push(b',');
                }
            }
            if n_lf != 0 {
                out.extend(b" LF");
                if n_nel != 0 {
                    out.push(b',');
                }
            }
            if n_nel != 0 {
                out.extend(b" NEL");
            }
        }
        out.extend(b" line terminators");
    }
    if has_escapes {
        out.extend(b", with escape sequences");
    }
    if has_backspace {
        out.extend(b", with overstriking");
    }
    Ok(true)
}
