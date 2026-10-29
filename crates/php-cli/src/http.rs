//! The HTTP request parser of PHP's built-in web server, as
//! sapi/cli/php_http_parser.c (the non-strict build) and the callbacks in
//! sapi/cli/php_cli_server.c use it: what it accepts, what it rejects, and
//! how headers are kept.

/// The methods the parser knows (php_http_method), by name.
const METHODS: &[&str] = &[
    "DELETE", "GET", "HEAD", "POST", "PUT", "PATCH", "CONNECT", "OPTIONS", "TRACE", "COPY", "LOCK", "MKCOL", "MOVE", "MKCALENDAR", "PROPFIND", "PROPPATCH", "SEARCH", "UNLOCK", "REPORT",
    "MKACTIVITY", "CHECKOUT", "MERGE", "M-SEARCH", "NOTIFY", "SUBSCRIBE", "UNSUBSCRIBE",
];

/// php_http_parser's limit on the request line and headers.
const MAX_HEADER_SIZE: usize = 80 * 1024;

/// A parsed request.
#[derive(Debug, Default)]
pub struct HttpRequest {
    /// php_http_method_str(): "NOTIMPLEMENTED" for a method the parser does not know.
    pub method: &'static str,
    pub not_implemented: bool,
    /// major * 100 + minor (101 for HTTP/1.1; 9 for a bare request line).
    pub protocol: u32,
    /// The URL as sent, query and fragment included.
    pub request_uri: Vec<u8>,
    /// The path part, before decoding.
    pub path: Option<Vec<u8>>,
    pub query: Option<Vec<u8>>,
    /// Headers by lowercase name; a repeat is joined with ", " (a
    /// Set-Cookie replaces).
    pub headers: Vec<(Vec<u8>, Vec<u8>)>,
    /// The same by the name as sent.
    pub headers_original: Vec<(Vec<u8>, Vec<u8>)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn header(&self, lowercase_name: &str) -> Option<&[u8]> {
        self.headers.iter().find(|(k, _)| k == lowercase_name.as_bytes()).map(|(_, v)| v.as_slice())
    }

    fn save_header(&mut self, name: Vec<u8>, value: Vec<u8>) {
        let lower = name.to_ascii_lowercase();
        let merged = match self.headers.iter_mut().find(|(k, _)| *k == lower) {
            Some((_, old)) => {
                if lower != b"set-cookie" {
                    let mut joined = old.clone();
                    joined.extend_from_slice(b", ");
                    joined.extend_from_slice(&value);
                    *old = joined.clone();
                    joined
                } else {
                    *old = value.clone();
                    value
                }
            }
            None => {
                self.headers.push((lower, value.clone()));
                value
            }
        };
        match self.headers_original.iter_mut().find(|(k, _)| *k == name) {
            Some((_, v)) => *v = merged,
            None => self.headers_original.push((name, merged)),
        }
    }
}

/// What reading the bytes so far gave.
pub enum Parse {
    /// A whole request, and how many bytes it took.
    Complete(Box<HttpRequest>),
    /// More bytes are needed; `at_start` when nothing but line breaks came yet.
    Incomplete,
    /// The parser's error.
    Malformed,
}

/// normal_url_char: 33..=126 except '#' and '?'.
fn normal_url_char(c: u8) -> bool {
    (33..=126).contains(&c) && c != b'#' && c != b'?'
}

/// The tokens table: header name characters (a space is one).
fn token(c: u8) -> bool {
    matches!(c, b' ' | b'!' | b'"' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'/' | b'0'..=b'9' | b'A'..=b'Z' | b'^' | b'_' | b'`' | b'a'..=b'z' | b'|' | b'}' | b'~')
}

/// A cursor over the bytes that says "need more" when they run out.
struct Cur<'a> {
    buf: &'a [u8],
    pos: usize,
    /// How far the request line and headers may reach.
    limit: usize,
}

enum Stop {
    More,
    Bad,
}

type R<T> = Result<T, Stop>;

impl Cur<'_> {
    fn peek(&self) -> R<u8> {
        self.buf.get(self.pos).copied().ok_or(Stop::More)
    }

    fn next(&mut self) -> R<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if self.pos > self.limit {
            return Err(Stop::Bad);
        }
        Ok(c)
    }
}

/// Read a request from `buf`.
pub fn parse(buf: &[u8]) -> Parse {
    let mut cur = Cur { buf, pos: 0, limit: MAX_HEADER_SIZE + 1 };
    match request(&mut cur) {
        Ok(r) => Parse::Complete(Box::new(r)),
        Err(Stop::Bad) => Parse::Malformed,
        Err(Stop::More) => Parse::Incomplete,
    }
}

fn request(cur: &mut Cur) -> R<HttpRequest> {
    let mut req = HttpRequest::default();
    // s_start_req: line breaks are skipped.
    let first = loop {
        let c = cur.next()?;
        if c != b'\r' && c != b'\n' {
            break c;
        }
    };
    if !first.is_ascii_uppercase() {
        return Err(Stop::Bad);
    }
    let mut method = vec![first];
    loop {
        let c = cur.next()?;
        if c == 0 {
            return Err(Stop::Bad);
        }
        if c == b' ' {
            break;
        }
        method.push(c);
    }
    match METHODS.iter().find(|m| m.as_bytes() == method.as_slice()) {
        Some(m) => req.method = m,
        None => {
            req.method = "NOTIMPLEMENTED";
            req.not_implemented = true;
        }
    }
    // The URL.
    let mut c = cur.next()?;
    while c == b' ' {
        c = cur.next()?;
    }
    let url_start = cur.pos - 1;
    let path_start;
    if c == b'/' || c == b'*' {
        path_start = url_start;
    } else if c.is_ascii_alphabetic() {
        // scheme://host:port/path
        let mut c = cur.next()?;
        while c.is_ascii_alphabetic() {
            c = cur.next()?;
        }
        if c == b':' {
            cur.next()?;
            cur.next()?;
        } else if c != b'.' && !c.is_ascii_digit() {
            return Err(Stop::Bad);
        }
        // s_req_host
        let mut c = cur.next()?;
        loop {
            if c.is_ascii_alphanumeric() || c == b'.' || c == b'-' {
                c = cur.next()?;
                continue;
            }
            match c {
                b':' => {
                    c = cur.next()?;
                    while c.is_ascii_digit() {
                        c = cur.next()?;
                    }
                    break;
                }
                b'/' | b' ' => break,
                _ => return Err(Stop::Bad),
            }
        }
        match c {
            b'/' => path_start = cur.pos - 1,
            b' ' => {
                // No path: "GET http://host HTTP/1.1".
                req.request_uri = cur.buf[url_start..cur.pos - 1].to_vec();
                return version_and_rest(cur, req);
            }
            _ => return Err(Stop::Bad),
        }
    } else {
        return Err(Stop::Bad);
    }
    // s_req_path: from path_start.
    let mut end_char;
    let path_end;
    loop {
        let c = cur.next()?;
        if normal_url_char(c) {
            continue;
        }
        match c {
            b' ' | b'\r' | b'\n' | b'?' | b'#' => {
                end_char = c;
                path_end = cur.pos - 1;
                break;
            }
            _ => return Err(Stop::Bad),
        }
    }
    req.path = Some(cur.buf[path_start..path_end].to_vec());
    if end_char == b'?' {
        // s_req_query_string_start: extra '?' are skipped.
        let mut query_start = None;
        loop {
            let c = cur.next()?;
            if normal_url_char(c) || c == b'?' {
                if query_start.is_none() {
                    if c == b'?' {
                        continue;
                    }
                    query_start = Some(cur.pos - 1);
                }
                continue;
            }
            match c {
                b' ' | b'\r' | b'\n' | b'#' => {
                    end_char = c;
                    if let Some(s) = query_start {
                        req.query = Some(cur.buf[s..cur.pos - 1].to_vec());
                    }
                    break;
                }
                _ => return Err(Stop::Bad),
            }
        }
    }
    if end_char == b'#' {
        // The fragment: part of the URL, nothing else.
        loop {
            let c = cur.next()?;
            if normal_url_char(c) || c == b'?' || c == b'#' {
                continue;
            }
            match c {
                b' ' | b'\r' | b'\n' => {
                    end_char = c;
                    break;
                }
                _ => return Err(Stop::Bad),
            }
        }
    }
    req.request_uri = cur.buf[url_start..cur.pos - 1].to_vec();
    match end_char {
        b' ' => version_and_rest(cur, req),
        b'\r' => {
            // HTTP/0.9: the line ends here.
            req.protocol = 9;
            if cur.next()? != b'\n' {
                return Err(Stop::Bad);
            }
            headers_and_body(cur, req)
        }
        _ => {
            req.protocol = 9;
            headers_and_body(cur, req)
        }
    }
}

/// After the URL's space: `HTTP/1.1` and the rest of the request.
fn version_and_rest(cur: &mut Cur, mut req: HttpRequest) -> R<HttpRequest> {
    let mut c = cur.next()?;
    while c == b' ' {
        c = cur.next()?;
    }
    if c != b'H' {
        return Err(Stop::Bad);
    }
    // "TTP/" is not checked.
    for _ in 0..4 {
        cur.next()?;
    }
    let digit = |c: u8| c.is_ascii_digit();
    let c = cur.next()?;
    if !(b'1'..=b'9').contains(&c) {
        return Err(Stop::Bad);
    }
    let mut major = u32::from(c - b'0');
    loop {
        let c = cur.next()?;
        if c == b'.' {
            break;
        }
        if !digit(c) {
            return Err(Stop::Bad);
        }
        major = major * 10 + u32::from(c - b'0');
        if major > 999 {
            return Err(Stop::Bad);
        }
    }
    let c = cur.next()?;
    if !digit(c) {
        return Err(Stop::Bad);
    }
    let mut minor = u32::from(c - b'0');
    loop {
        let c = cur.next()?;
        match c {
            b'\r' => {
                if cur.next()? != b'\n' {
                    return Err(Stop::Bad);
                }
                break;
            }
            b'\n' => break,
            c if digit(c) => {
                minor = minor * 10 + u32::from(c - b'0');
                if minor > 999 {
                    return Err(Stop::Bad);
                }
            }
            _ => return Err(Stop::Bad),
        }
    }
    req.protocol = major * 100 + minor;
    headers_and_body(cur, req)
}

/// Header lines until the blank one; returns the headers read and the
/// content-length / chunked facts.
fn header_block(cur: &mut Cur, req: &mut HttpRequest, flags: &mut (i64, bool)) -> R<()> {
    loop {
        let c = cur.next()?;
        if c == b'\r' {
            // The next byte is taken for the LF without a look.
            cur.next()?;
            return Ok(());
        }
        if c == b'\n' {
            return Ok(());
        }
        if !token(c) {
            return Err(Stop::Bad);
        }
        let name_start = cur.pos - 1;
        // The field name.
        let (name_end, ended_by) = loop {
            let c = cur.next()?;
            if token(c) {
                continue;
            }
            match c {
                b':' | b'\r' | b'\n' => break (cur.pos - 1, c),
                _ => return Err(Stop::Bad),
            }
        };
        let name = cur.buf[name_start..name_end].to_vec();
        let mut value = Vec::new();
        if ended_by == b'\r' {
            cur.next()?;
        } else if ended_by == b':' {
            let mut c = cur.next()?;
            while c == b' ' {
                c = cur.next()?;
            }
            let lower = name.to_ascii_lowercase();
            let is_length = lower == b"content-length";
            // A value runs to the line break.
            let value_start = cur.pos - 1;
            if is_length && !(c == b'\r' || c == b'\n') && !c.is_ascii_digit() {
                return Err(Stop::Bad);
            }
            loop {
                if c == b'\r' || c == b'\n' {
                    value = cur.buf[value_start..cur.pos - 1].to_vec();
                    if c == b'\r' {
                        cur.next()?;
                    }
                    break;
                }
                c = cur.next()?;
            }
            if is_length {
                let mut n: i64 = 0;
                let mut seen = false;
                for &d in &value {
                    if d == b' ' {
                        continue;
                    }
                    if !d.is_ascii_digit() {
                        return Err(Stop::Bad);
                    }
                    n = n.saturating_mul(10).saturating_add(i64::from(d - b'0'));
                    seen = true;
                }
                if seen {
                    flags.0 = n;
                }
            } else if lower == b"transfer-encoding" {
                let v = value.to_ascii_lowercase();
                let t = v.iter().rposition(|&c| c != b' ').map_or(0, |i| i + 1);
                if &v[..t] == b"chunked" {
                    flags.1 = true;
                }
            }
        }
        req.save_header(name, value);
    }
}

fn headers_and_body(cur: &mut Cur, mut req: HttpRequest) -> R<HttpRequest> {
    // (content length, chunked)
    let mut flags: (i64, bool) = (-1, false);
    header_block(cur, &mut req, &mut flags)?;
    cur.limit = usize::MAX;
    if flags.1 {
        // Chunked: sizes in hex, an optional extension, the data; 0 ends.
        loop {
            let c = cur.next()?;
            let Some(mut size) = (c as char).to_digit(16).map(i64::from) else { return Err(Stop::Bad) };
            loop {
                let c = cur.next()?;
                if c == b'\r' {
                    break;
                }
                match (c as char).to_digit(16) {
                    Some(d) => size = size.saturating_mul(16).saturating_add(i64::from(d)),
                    None if c == b';' || c == b' ' => {
                        while cur.next()? != b'\r' {}
                        break;
                    }
                    None => return Err(Stop::Bad),
                }
            }
            cur.next()?; // the LF
            if size == 0 {
                // Trailers, then the end.
                let mut trailer = HttpRequest::default();
                header_block(cur, &mut trailer, &mut (-1, false))?;
                return Ok(req);
            }
            let size = size as usize;
            if cur.buf.len() < cur.pos + size {
                return Err(Stop::More);
            }
            req.body.extend_from_slice(&cur.buf[cur.pos..cur.pos + size]);
            cur.pos += size;
            cur.next()?; // CR
            cur.next()?; // LF
        }
    }
    if flags.0 > 0 {
        let n = flags.0 as usize;
        if cur.buf.len() < cur.pos + n {
            return Err(Stop::More);
        }
        req.body = cur.buf[cur.pos..cur.pos + n].to_vec();
        cur.pos += n;
    }
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(bytes: &[u8]) -> HttpRequest {
        match parse(bytes) {
            Parse::Complete(r) => *r,
            Parse::Incomplete => panic!("incomplete"),
            Parse::Malformed => panic!("malformed"),
        }
    }

    #[test]
    fn reads_a_request_line_and_headers() {
        let r = complete(b"GET /a/b.php?x=1&y=2#frag HTTP/1.1\r\nHost: localhost:8080\r\nX-A: 1\r\nx-a: 2\r\n\r\n");
        assert_eq!(r.method, "GET");
        assert_eq!(r.protocol, 101);
        assert_eq!(r.path.as_deref(), Some(&b"/a/b.php"[..]));
        assert_eq!(r.query.as_deref(), Some(&b"x=1&y=2"[..]));
        assert_eq!(r.request_uri, b"/a/b.php?x=1&y=2#frag");
        assert_eq!(r.header("host"), Some(&b"localhost:8080"[..]));
        // A repeated header joins; the original-case table keeps both spellings.
        assert_eq!(r.header("x-a"), Some(&b"1, 2"[..]));
        let spelled: Vec<&[u8]> = r.headers_original.iter().map(|(k, _)| k.as_slice()).collect();
        assert_eq!(spelled, [&b"Host"[..], b"X-A", b"x-a"]);
    }

    #[test]
    fn reads_bodies() {
        let r = complete(b"POST / HTTP/1.0\r\nContent-Length: 5\r\n\r\nhello");
        assert_eq!((r.protocol, r.body.as_slice()), (100, &b"hello"[..]));
        let r = complete(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n");
        assert_eq!(r.body, b"abcde");
        assert!(matches!(parse(b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\nhel"), Parse::Incomplete));
        assert!(matches!(parse(b"\r\n"), Parse::Incomplete));
    }

    #[test]
    fn rejects_what_the_parser_rejects() {
        assert!(matches!(parse(b"get / HTTP/1.1\r\n\r\n"), Parse::Malformed));
        assert!(matches!(parse(b"GET /\xc3\xa9 HTTP/1.1\r\n\r\n"), Parse::Malformed));
        assert!(matches!(parse(b"GET / HTTP/1.1\r\nContent-Length: x\r\n\r\n"), Parse::Malformed));
        let r = complete(b"FOO / HTTP/1.1\r\n\r\n");
        assert!(r.not_implemented && r.method == "NOTIMPLEMENTED");
    }
}
