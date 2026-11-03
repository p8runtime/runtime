//! URIs as libxml2 handles them (uri.c): RFC 3986 resolution against a
//! base (xmlBuildURI), the relative form (xmlBuildRelativeURI), and the
//! local file a URI names. XInclude and XML Schema imports resolve their
//! hrefs and schemaLocations this way.

pub(crate) struct Uri<'a> {
    pub scheme: Option<&'a str>,
    pub authority: Option<&'a str>,
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub fragment: Option<&'a str>,
}

pub(crate) fn split(s: &str) -> Uri<'_> {
    let (s, fragment) = match s.find('#') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (s, query) = match s.find('?') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let mut scheme = None;
    let mut rest = s;
    if let Some(i) = s.find(':')
        && i > 0
        && s[..i].starts_with(|c: char| c.is_ascii_alphabetic())
        && s[..i].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        scheme = Some(&s[..i]);
        rest = &s[i + 1..];
    }
    let mut authority = None;
    if let Some(r) = rest.strip_prefix("//") {
        let end = r.find('/').unwrap_or(r.len());
        authority = Some(&r[..end]);
        rest = &r[end..];
    }
    Uri { scheme, authority, path: rest, query, fragment }
}

pub(crate) fn assemble(scheme: Option<&str>, authority: Option<&str>, path: &str, query: Option<&str>, fragment: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(s) = scheme {
        out.push_str(s);
        out.push(':');
    }
    if let Some(a) = authority {
        out.push_str("//");
        out.push_str(a);
    }
    out.push_str(path);
    if let Some(q) = query {
        out.push('?');
        out.push_str(q);
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Removes . and .. segments.
pub(crate) fn normalize(path: &str) -> String {
    let abs = path.starts_with('/');
    let body = if abs { &path[1..] } else { path };
    let segs: Vec<&str> = body.split('/').collect();
    let mut out: Vec<&str> = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        match *s {
            "." => {
                if last {
                    out.push("");
                }
            }
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else if !abs {
                    out.push("..");
                }
                if last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let joined = out.join("/");
    if abs { format!("/{joined}") } else { joined }
}

/// `reference` resolved against `base` (RFC 3986 as libxml2 does it).
pub(crate) fn resolve(reference: &str, base: &str) -> String {
    if base.is_empty() {
        return reference.to_string();
    }
    let r = split(reference);
    if r.scheme.is_some() {
        return reference.to_string();
    }
    let b = split(base);
    if r.authority.is_some() {
        return assemble(b.scheme, r.authority, &normalize(r.path), r.query, r.fragment);
    }
    if r.path.is_empty() {
        return assemble(b.scheme, b.authority, b.path, r.query.or(b.query), r.fragment);
    }
    if r.path.starts_with('/') {
        return assemble(b.scheme, b.authority, &normalize(r.path), r.query, r.fragment);
    }
    let merged = match b.path.rfind('/') {
        Some(i) => format!("{}{}", &b.path[..=i], r.path),
        None if b.authority.is_some() => format!("/{}", r.path),
        None => r.path.to_string(),
    };
    assemble(b.scheme, b.authority, &normalize(&merged), r.query, r.fragment)
}

/// `uri` relative to `base` where they share scheme and host; else `uri`.
pub(crate) fn relative(uri: &str, base: &str) -> String {
    if uri.is_empty() || base.is_empty() {
        return uri.to_string();
    }
    let (u, b) = (split(uri), split(base));
    if u.scheme != b.scheme || u.authority != b.authority || !u.path.starts_with('/') || !b.path.starts_with('/') {
        return uri.to_string();
    }
    let (ub, bb) = (u.path.as_bytes(), b.path.as_bytes());
    let mut common = 0;
    let mut i = 0;
    while i < ub.len() && i < bb.len() && ub[i] == bb[i] {
        if ub[i] == b'/' {
            common = i + 1;
        }
        i += 1;
    }
    let ups = b.path[common..].matches('/').count();
    let mut out = "../".repeat(ups);
    out.push_str(&u.path[common..]);
    if ups == 0 && out.split('/').next().is_some_and(|s| s.contains(':')) {
        out.insert_str(0, "./");
    }
    if let Some(q) = u.query {
        out.push('?');
        out.push_str(q);
    }
    out
}

/// The bytes a URL names: a local file (plain path, file:, or PHP's
/// compress.zlib: wrapper), None for what libxml2 could not load.
pub(crate) fn read_url(url: &str) -> Option<Vec<u8>> {
    std::fs::read(local_path(url)?).ok()
}

/// The local file a URL names: a plain path, a file: URI (percent-decoded,
/// localhost allowed), or PHP's compress.zlib: wrapper; None for any other
/// scheme (libxml2 here loads no network resources).
pub(crate) fn local_path(url: &str) -> Option<String> {
    Some(match split(url).scheme {
        None => url.to_string(),
        Some(s) if s.eq_ignore_ascii_case("file") => {
            let rest = url[s.len() + 1..].strip_prefix("//").map(|r| r.strip_prefix("localhost").unwrap_or(r)).unwrap_or(&url[s.len() + 1..]);
            percent_decode(rest)
        }
        Some("compress.zlib") => url["compress.zlib://".len()..].to_string(),
        Some(_) => return None,
    })
}

pub(crate) fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_like_libxml2() {
        assert_eq!(resolve("part.xml", "/work/main.xml"), "/work/part.xml");
        assert_eq!(resolve("../a/b.xml", "/work/sub/main.xml"), "/work/a/b.xml");
        assert_eq!(resolve("/x/y.xml", "http://h/dir/f.xml"), "http://h/x/y.xml");
        assert_eq!(resolve("", "/work/main.xml"), "/work/main.xml");
        assert_eq!(resolve("file:///a", "/b/"), "file:///a");
    }

    #[test]
    fn relative_goes_up_and_down() {
        assert_eq!(relative("/work/part.xml", "/"), "work/part.xml");
        assert_eq!(relative("/work/part.xml", "/work/main.xml"), "part.xml");
        assert_eq!(relative("/work/part.xml", "/work/sub/main.xml"), "../part.xml");
        assert_eq!(relative("/work/part.xml", "/a/b/main.xml"), "../../work/part.xml");
        assert_eq!(relative("file:///w/p.xml", "/"), "file:///w/p.xml");
    }

    #[test]
    fn file_uris_name_local_paths() {
        assert_eq!(local_path("file:///a/b%20c/x.xsd").as_deref(), Some("/a/b c/x.xsd"));
        assert_eq!(local_path("file://localhost/a/x.xsd").as_deref(), Some("/a/x.xsd"));
        assert_eq!(local_path("/a/x.xsd").as_deref(), Some("/a/x.xsd"));
        assert_eq!(local_path("http://www.w3.org/2001/xml.xsd"), None);
        assert_eq!(percent_decode("a%2F%41"), "a/A");
    }
}
