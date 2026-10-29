//! Host names: gethostname, gethostbyname, gethostbynamel, gethostbyaddr,
//! through the system resolver (getaddrinfo, getnameinfo) as PHP's are.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/dns.c

use crate::arg;
use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "gethostname", run: gethostname, by_ref: &[] },
    Builtin { name: "inet_pton", run: inet_pton, by_ref: &[] },
    Builtin { name: "inet_ntop", run: inet_ntop, by_ref: &[] },
    Builtin { name: "gethostbyname", run: gethostbyname, by_ref: &[] },
    Builtin { name: "gethostbynamel", run: gethostbynamel, by_ref: &[] },
    Builtin { name: "gethostbyaddr", run: gethostbyaddr, by_ref: &[] },
];

/// MAXFQDNLEN: longer names are refused before they reach the resolver.
const MAX_NAME: usize = 255;

fn path_arg(ctx: &mut dyn Ctx, args: &[Value], func: &str, name: &str) -> Result<String, Throw> {
    let v = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    if v.as_bytes().contains(&0) {
        return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 (${name}) must not contain any null bytes")));
    }
    Ok(String::from_utf8_lossy(v.as_bytes()).into_owned())
}

fn gethostname(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(dns_lookup::get_hostname().map_or(Value::Bool(false), Value::str))
}

/// The IPv4 addresses `name` resolves to, in the resolver's order.
fn ipv4s(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in (name, 0).to_socket_addrs().into_iter().flatten() {
        if let IpAddr::V4(v4) = a.ip()
            && !out.contains(&v4.to_string())
        {
            out.push(v4.to_string());
        }
    }
    out
}

/// gethostbyname($hostname): its first IPv4 address, or the name unchanged.
fn gethostbyname(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = path_arg(ctx, args, "gethostbyname", "hostname")?;
    if name.len() > MAX_NAME {
        ctx.warn(&format!("gethostbyname(): Host name cannot be longer than {MAX_NAME} characters"));
        return Ok(Value::str(name));
    }
    Ok(Value::str(ipv4s(&name).into_iter().next().unwrap_or(name)))
}

/// gethostbynamel($hostname): its IPv4 addresses, false when it has none.
fn gethostbynamel(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let name = path_arg(ctx, args, "gethostbynamel", "hostname")?;
    if name.len() > MAX_NAME {
        ctx.warn(&format!("gethostbynamel(): Host name cannot be longer than {MAX_NAME} characters"));
        return Ok(Value::Bool(false));
    }
    let found = ipv4s(&name);
    if found.is_empty() {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Array(Rc::new(PhpArray::from_list(found.into_iter().map(Value::str).collect()))))
}

/// gethostbyaddr($ip): the name the address reverses to, the address
/// itself when it has none, false when it is not one.
fn gethostbyaddr(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let ip = path_arg(ctx, args, "gethostbyaddr", "ip")?;
    let Ok(addr) = ip.parse::<IpAddr>() else {
        ctx.warn("gethostbyaddr(): Address is not a valid IPv4 or IPv6 address");
        return Ok(Value::Bool(false));
    };
    // NI_NAMEREQD: no name is a failure, not the address as text.
    match dns_lookup::getnameinfo(&SocketAddr::new(addr, 0), libc::NI_NAMEREQD) {
        Ok((name, _)) => Ok(Value::str(name)),
        Err(_) => Ok(Value::str(ip)),
    }
}

/// inet_pton($ip): the address in binary (4 or 16 bytes), false when it is
/// not one (glibc's inet_pton: no leading zeros, IPv4 written in full).
fn inet_pton(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let text = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    let Ok(text) = std::str::from_utf8(text.as_bytes()) else { return Ok(Value::Bool(false)) };
    let bytes = if text.contains(':') {
        text.parse::<std::net::Ipv6Addr>().map(|a| a.octets().to_vec())
    } else {
        text.parse::<std::net::Ipv4Addr>().map(|a| a.octets().to_vec())
    };
    Ok(bytes.map_or(Value::Bool(false), |b| Value::Str(PhpStr::new(b))))
}

/// inet_ntop($ip): a 4- or 16-byte address as text, as glibc writes it.
fn inet_ntop(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let bin = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    Ok(match bin.as_bytes() {
        [a, b, c, d] => Value::Str(PhpStr::from(format!("{a}.{b}.{c}.{d}"))),
        b if b.len() == 16 => Value::Str(PhpStr::from(ipv6_text(b))),
        _ => Value::Bool(false),
    })
}

/// glibc's inet_ntop6: groups in lowercase hex without leading zeros, the
/// longest run of two or more zero groups (the first, on a tie) as `::`,
/// and an IPv4-mapped (::ffff:0:0/96) or IPv4-compatible address's last
/// four bytes as a dotted quad.
fn ipv6_text(b: &[u8]) -> String {
    let words: Vec<u16> = b.chunks(2).map(|w| u16::from_be_bytes([w[0], w[1]])).collect();
    let (mut best, mut cur): ((usize, usize), Option<(usize, usize)>) = ((0, 0), None);
    for (i, &w) in words.iter().enumerate() {
        if w == 0 {
            let run = cur.map_or((i, 1), |(s, l)| (s, l + 1));
            cur = Some(run);
            if run.1 > best.1 {
                best = run;
            }
        } else {
            cur = None;
        }
    }
    let zeros = if best.1 >= 2 { Some(best) } else { None };
    let mut out = String::new();
    let mut i = 0;
    while i < 8 {
        if let Some((start, len)) = zeros
            && i == start
        {
            out.push(':');
            if start == 0 {
                out.push(':');
            }
            i += len;
            continue;
        }
        // The last two groups of a mapped or compatible address: a.b.c.d.
        if i == 6 && zeros.is_some_and(|(s, l)| s == 0 && (l == 6 || (l == 5 && words[5] == 0xffff))) {
            out.push_str(&format!("{}.{}.{}.{}", b[12], b[13], b[14], b[15]));
            break;
        }
        out.push_str(&format!("{:x}", words[i]));
        if i < 7 {
            out.push(':');
        }
        i += 1;
    }
    out
}
