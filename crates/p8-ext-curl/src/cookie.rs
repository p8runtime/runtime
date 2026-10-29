//! CURLOPT_COOKIELIST and CURLINFO_COOKIELIST: the handle's cookie jar as
//! libcurl's Netscape lines.

use crate::{State, c, lossy};
use rsurl::{Cookie, CookieJar, SameSite};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The jar, made (empty) if the handle has none yet.
fn jar(st: &mut State) -> Rc<RefCell<CookieJar>> {
    Rc::clone(st.jar.get_or_insert_with(|| Rc::new(RefCell::new(CookieJar::new()))))
}

/// A cookie as libcurl lists it: domain, subdomains flag, path, secure,
/// expiry, name, value (tab separated).
fn line(cookie: &Cookie) -> String {
    let prefix = if cookie.http_only { "#HttpOnly_" } else { "" };
    let dot = if cookie.host_only { "" } else { "." };
    format!(
        "{prefix}{dot}{}\t{}\t{}\t{}\t{}\t{}\t{}",
        cookie.domain,
        if cookie.host_only { "FALSE" } else { "TRUE" },
        cookie.path,
        if cookie.secure { "TRUE" } else { "FALSE" },
        cookie.expires.unwrap_or(0),
        cookie.name,
        cookie.value
    )
}

/// CURLINFO_COOKIELIST.
pub(crate) fn list(st: &State) -> Vec<String> {
    match &st.jar {
        Some(j) => j.borrow().iter().map(line).collect(),
        None => Vec::new(),
    }
}

fn attribute<'a>(set_cookie: &'a str, name: &str) -> Option<&'a str> {
    set_cookie.split(';').skip(1).find_map(|a| {
        let (k, v) = a.split_once('=')?;
        k.trim().eq_ignore_ascii_case(name).then_some(v.trim())
    })
}

/// CURLOPT_COOKIELIST: a command ("ALL", "SESS", "FLUSH", "RELOAD"), a
/// Set-Cookie line, or a Netscape line.
pub(crate) fn command(st: &mut State, cmd: &[u8]) {
    let cmd = lossy(cmd);
    let jar = jar(st);
    match cmd.as_str() {
        "ALL" => jar.borrow_mut().clear(),
        "SESS" => {
            let session: Vec<(String, String, String)> = jar.borrow().iter().filter(|c| c.expires.is_none()).map(|c| (c.name.clone(), c.domain.clone(), c.path.clone())).collect();
            for (n, d, p) in session {
                jar.borrow_mut().remove(&n, &d, &p);
            }
        }
        "FLUSH" => {
            if let Some(path) = st.opts.get(&c::CURLOPT_COOKIEJAR).map(|v| lossy(&crate::bytes_of(v))).filter(|p| !p.is_empty()) {
                let _ = jar.borrow().save_netscape(&path);
            }
        }
        "RELOAD" => {
            if let Some(path) = st.opts.get(&c::CURLOPT_COOKIEFILE).map(|v| lossy(&crate::bytes_of(v))).filter(|p| !p.is_empty())
                && let Ok(loaded) = CookieJar::load_netscape_or_empty(&path)
            {
                *jar.borrow_mut() = loaded;
            }
        }
        _ => {
            let lower = cmd.to_ascii_lowercase();
            if lower.starts_with("set-cookie:") {
                let value = cmd["set-cookie:".len()..].trim();
                // The cookie belongs to the domain it names (there is no request).
                let host = attribute(value, "domain").map_or("localhost".to_string(), |d| d.trim_start_matches('.').to_string());
                if let Ok(url) = rsurl::Url::parse(&format!("http://{host}/")) {
                    jar.borrow_mut().add_set_cookie(&url, value, now());
                }
            } else {
                let f: Vec<&str> = cmd.split('\t').collect();
                if f.len() >= 7 {
                    let (http_only, domain) = match f[0].strip_prefix("#HttpOnly_") {
                        Some(d) => (true, d),
                        None => (false, f[0]),
                    };
                    let host_only = !f[1].eq_ignore_ascii_case("TRUE");
                    let expires = f[4].parse::<u64>().ok().filter(|e| *e > 0);
                    jar.borrow_mut().insert(Cookie {
                        name: f[5].to_string(),
                        value: f[6].to_string(),
                        domain: domain.trim_start_matches('.').to_ascii_lowercase(),
                        path: f[2].to_string(),
                        expires,
                        secure: f[3].eq_ignore_ascii_case("TRUE"),
                        http_only,
                        host_only,
                        same_site: SameSite::Unspecified,
                        partitioned: false,
                        partition_key: None,
                    });
                }
            }
        }
    }
}
