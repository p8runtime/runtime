//! Scan opaque JS source (an `unsafe { }` body) without a full JS parser.

/// True when `source` uses `await` / `for await` in the **wrapper** body.
/// Nested `function` / `=> { }` bodies and strings/comments do not count.
///
/// Used to pick a sync vs async `unsafe` IIFE (RFD 21).
pub fn js_has_top_level_await(source: &str) -> bool {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut brace_depth: i32 = 0;
    let mut nested_fn_depth: i32 = 0;
    let mut pending_fn = false;
    let mut fn_brace_at: Vec<i32> = Vec::new();

    while i < n {
        let c = bytes[i];

        if c == b'/' && i + 1 < n {
            if bytes[i + 1] == b'/' {
                i += 2;
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n);
                continue;
            }
        }

        if c == b'\'' || c == b'"' {
            i = skip_quoted(bytes, i, c);
            continue;
        }
        if c == b'`' {
            i = skip_template(bytes, i);
            continue;
        }

        if is_ident_start(c) {
            let start = i;
            i += 1;
            while i < n && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let ident = &source[start..i];
            if ident == "function" || ident == "class" {
                pending_fn = true;
                continue;
            }
            if ident == "await" && nested_fn_depth == 0 && !pending_fn {
                return true;
            }
            if ident == "async" {
                // `async function` / `async () =>` starts a nested async fn.
                pending_fn = true;
            }
            continue;
        }

        if c == b'=' && i + 1 < n && bytes[i + 1] == b'>' {
            pending_fn = true;
            i += 2;
            skip_ws(bytes, &mut i);
            if i < n && is_ident_start(bytes[i]) {
                let start = i;
                i += 1;
                while i < n && is_ident_continue(bytes[i]) {
                    i += 1;
                }
                if &source[start..i] == "await" {
                    // Concise arrow: `() => await x` — nested, not wrapper body.
                    pending_fn = false;
                    continue;
                }
            }
            continue;
        }

        if c == b'{' {
            brace_depth += 1;
            if pending_fn {
                fn_brace_at.push(brace_depth);
                nested_fn_depth += 1;
                pending_fn = false;
            }
            i += 1;
            continue;
        }
        if c == b'}' {
            if fn_brace_at.last().copied() == Some(brace_depth) {
                fn_brace_at.pop();
                nested_fn_depth -= 1;
            }
            brace_depth -= 1;
            i += 1;
            continue;
        }

        if !c.is_ascii_whitespace() {
            // `() => 1` without `{`: drop pending so later `await` at depth 0 counts.
            if pending_fn && c != b'(' && c != b')' && c != b',' {
                pending_fn = false;
            }
        }
        i += 1;
    }
    false
}

fn skip_ws(bytes: &[u8], i: &mut usize) {
    while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn skip_template(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$'
}

fn is_ident_continue(c: u8) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::js_has_top_level_await;

    #[test]
    fn expression_await() {
        assert!(js_has_top_level_await("await fetch(url)"));
        assert!(js_has_top_level_await("await fetch(url);"));
        assert!(js_has_top_level_await("for await (const x of y) {}"));
    }

    #[test]
    fn nested_function_await_does_not_count() {
        assert!(!js_has_top_level_await(
            "async function load() { return await fetch(url) } return load()"
        ));
        assert!(!js_has_top_level_await("() => await fetch(url)"));
        assert!(!js_has_top_level_await("() => { return await fetch(url) }"));
    }

    #[test]
    fn strings_and_comments() {
        assert!(!js_has_top_level_await("'await fetch'"));
        assert!(!js_has_top_level_await("\"await\""));
        assert!(!js_has_top_level_await("// await fetch\nJSON.parse(s)"));
        assert!(!js_has_top_level_await("/* await fetch */ 1"));
        assert!(!js_has_top_level_await("`await ${x}`"));
    }

    #[test]
    fn sync_bodies() {
        assert!(!js_has_top_level_await("JSON.parse(s)"));
        assert!(!js_has_top_level_await("fetch(url)"));
        assert!(!js_has_top_level_await("const x = 1; return x;"));
    }
}
