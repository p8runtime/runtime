//! What PHP's built-in web server (the cli-server SAPI) adds to the CLI:
//! apache_request_headers() and getallheaders(), apache_response_headers(),
//! and `PHP_SAPI` being "cli-server". A host serving web requests
//! registers this extension after the standard one.
//!
//! Reference: $PHP_SRC_PATH/sapi/cli/php_cli_server.c

use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

/// The key the host keeps the request's headers (original case) under.
pub const REQUEST_HEADERS: &str = "cli_server_request_headers";

static BUILTINS: &[Builtin] = &[
    Builtin { name: "apache_request_headers", run: apache_request_headers, by_ref: &[] },
    Builtin { name: "getallheaders", run: apache_request_headers, by_ref: &[] },
    Builtin { name: "apache_response_headers", run: apache_response_headers, by_ref: &[] },
];

/// The cli-server SAPI's additions to the standard extension.
pub const EXTENSION: Extension = Extension { name: "cli_server", register };

fn register(registry: &mut Registry) {
    registry.add_constant(b"PHP_SAPI", Constant::Str(b"cli-server"));
    registry.add_functions(BUILTINS);
    registry.add_module("cli_server");
}

/// The request's headers, as the client wrote their names.
fn apache_request_headers(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(match ctx.state(REQUEST_HEADERS).clone() {
        v @ Value::Array(_) => v,
        _ => Value::Array(Rc::new(PhpArray::new())),
    })
}

/// The response headers set so far, by name (a later one of a name wins).
fn apache_response_headers(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let mut out = PhpArray::new();
    for h in ctx.response().headers.iter() {
        let Some(colon) = h.iter().position(|&c| c == b':').filter(|&c| c > 0) else { continue };
        let mut end = colon;
        while end > 0 && matches!(h[end - 1], b' ' | b'\t') {
            end -= 1;
        }
        if end == 0 {
            continue;
        }
        let mut value = colon + 1;
        while value < h.len() && matches!(h[value], b' ' | b'\t') {
            value += 1;
        }
        out.insert(Key::from_string(PhpStr::from(&h[..end])), Value::Str(PhpStr::from(&h[value..])));
    }
    Ok(Value::Array(Rc::new(out)))
}
