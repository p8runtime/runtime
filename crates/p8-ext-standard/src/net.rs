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
