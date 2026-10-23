use crate::core::value::{ArrayData, ArrayKey, Handle, Val};
use crate::vm::engine::VM;
use pcre2::bytes::{Captures, Regex, RegexBuilder};
use smallvec::smallvec;
use std::rc::Rc;

pub fn preg_match(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // args: pattern, subject, matches (ref), flags, offset
    if args.len() < 2 {
        return Err("preg_match expects at least 2 arguments".into());
    }

    let pattern_handle = args[0];
    let subject_handle = args[1];

    let pattern_str = match &vm.arena.get(pattern_handle).value {
        Val::String(s) => s.clone(),
        _ => return Err("preg_match pattern must be a string".into()),
    };

    let subject_str = match &vm.arena.get(subject_handle).value {
        Val::String(s) => s.clone(),
        _ => Rc::new(
            vm.convert_to_string(subject_handle)
                .map_err(|e| e.to_string())?,
        ),
    };

    let Some(regex) = compile_or_warn(vm, "preg_match", &pattern_str) else {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    };

    // If matches array is provided, populate it
    if args.len() >= 3 {
        let matches_handle = args[2];
        let captures = regex
            .captures(&subject_str)
            .map_err(|e| format!("Regex execution error: {}", e))?;

        if let Some(captures) = captures {
            let mut match_array = ArrayData::new();
            let mut indexed_handles = Vec::new();

            for i in 0..captures.len() {
                let handle = if let Some(m) = captures.get(i) {
                    let match_str = m.as_bytes().to_vec();
                    vm.arena.alloc(Val::String(Rc::new(match_str)))
                } else {
                    vm.arena.alloc(Val::String(Rc::new(Vec::new())))
                };
                match_array.insert(ArrayKey::Int(i as i64), handle);
                indexed_handles.push(handle);
            }

            for (idx, name) in regex.capture_names().iter().enumerate() {
                if let Some(name) = name.as_deref()
                    && let Some(&handle) = indexed_handles.get(idx)
                {
                    match_array.insert(ArrayKey::Str(Rc::new(name.as_bytes().to_vec())), handle);
                }
            }

            // Update the referenced matches variable
            if vm.arena.get(matches_handle).is_ref {
                let slot = vm.arena.get_mut(matches_handle);
                slot.value = Val::Array(Rc::new(match_array));
            }

            // Match found
            Ok(vm.arena.alloc(Val::Int(1)))
        } else {
            // No match
            Ok(vm.arena.alloc(Val::Int(0)))
        }
    } else {
        let is_match = regex
            .is_match(&subject_str)
            .map_err(|e| format!("Regex execution error: {}", e))?;
        Ok(vm.arena.alloc(Val::Int(if is_match { 1 } else { 0 })))
    }
}

pub fn preg_replace(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // args: pattern, replacement, subject, limit, count
    if args.len() < 3 {
        return Err("preg_replace expects at least 3 arguments".into());
    }

    let pattern_handle = args[0];
    let replacement_handle = args[1];
    let subject_handle = args[2];

    let limit = if args.len() >= 4 {
        match vm.arena.get(args[3]).value {
            Val::Int(l) => l,
            _ => -1,
        }
    } else {
        -1
    };

    let pattern_str = match &vm.arena.get(pattern_handle).value {
        Val::String(s) => s.clone(),
        _ => return Err("preg_replace pattern must be a string".into()),
    };

    let replacement_str = match &vm.arena.get(replacement_handle).value {
        Val::String(s) => s.clone(),
        _ => return Err("preg_replace replacement must be a string".into()),
    };

    let subject_str = match &vm.arena.get(subject_handle).value {
        Val::String(s) => s.clone(),
        _ => return Err("preg_replace subject must be a string".into()),
    };

    let Some(regex) = compile_or_warn(vm, "preg_replace", &pattern_str) else {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    };

    let mut result = Vec::new();
    let mut last_end = 0;
    let mut count = 0;

    for captures in regex.captures_iter(&subject_str) {
        let captures = captures.map_err(|e| format!("Regex error: {}", e))?;

        // captures.get(0) is the whole match
        if let Some(m) = captures.get(0) {
            if limit != -1 && count >= limit {
                break;
            }

            result.extend_from_slice(&subject_str[last_end..m.start()]);

            let replaced = interpolate_replacement(&replacement_str, &captures);
            result.extend_from_slice(&replaced);

            last_end = m.end();
            count += 1;
        }
    }

    result.extend_from_slice(&subject_str[last_end..]);

    // Update count variable if provided
    if args.len() >= 5 {
        let count_handle = args[4];
        if vm.arena.get(count_handle).is_ref {
            let slot = vm.arena.get_mut(count_handle);
            slot.value = Val::Int(count);
        }
    }

    Ok(vm.arena.alloc(Val::String(Rc::new(result))))
}

pub fn preg_match_all(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // args: pattern, subject, matches (ref), flags, offset
    if args.len() < 2 {
        return Err("preg_match_all expects at least 2 arguments".into());
    }

    let pattern_handle = args[0];
    let subject_handle = args[1];

    let flags = if args.len() >= 4 {
        match vm.arena.get(args[3]).value {
            Val::Int(value) => value,
            _ => 0,
        }
    } else {
        0
    };

    let offset = if args.len() >= 5 {
        match vm.arena.get(args[4]).value {
            Val::Int(value) if value > 0 => value as usize,
            _ => 0,
        }
    } else {
        0
    };

    let pattern_str = match &vm.arena.get(pattern_handle).value {
        Val::String(s) => s.clone(),
        _ => return Err("preg_match_all pattern must be a string".into()),
    };

    let subject_str = match &vm.arena.get(subject_handle).value {
        Val::String(s) => s.clone(),
        _ => Rc::new(
            vm.convert_to_string(subject_handle)
                .map_err(|e| e.to_string())?,
        ),
    };

    let Some(regex) = compile_or_warn(vm, "preg_match_all", &pattern_str) else {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    };

    let set_order = (flags & 2) != 0;
    let offset_capture = (flags & (1 << 8)) != 0;
    let unmatched_as_null = (flags & (1 << 9)) != 0;

    let mut count = 0i64;
    let mut matches_array = ArrayData::new();

    if set_order {
        let mut match_index = 0i64;
        for captures in regex.captures_iter(&subject_str) {
            let captures = captures.map_err(|e| format!("Regex execution error: {}", e))?;
            let Some(m0) = captures.get(0) else {
                continue;
            };
            if m0.start() < offset {
                continue;
            }

            let mut row = ArrayData::new();
            for i in 0..captures.len() {
                let capture = captures.get(i);
                let val = capture_to_val(vm, capture, offset_capture, unmatched_as_null);
                row.insert(ArrayKey::Int(i as i64), val);
            }

            let row_handle = vm.arena.alloc(Val::Array(Rc::new(row)));
            matches_array.insert(ArrayKey::Int(match_index), row_handle);
            match_index += 1;
            count += 1;
        }
    } else {
        let mut group_arrays: Vec<ArrayData> = Vec::new();
        let mut match_index = 0i64;

        for captures in regex.captures_iter(&subject_str) {
            let captures = captures.map_err(|e| format!("Regex execution error: {}", e))?;
            let Some(m0) = captures.get(0) else {
                continue;
            };
            if m0.start() < offset {
                continue;
            }

            if group_arrays.is_empty() {
                group_arrays = (0..captures.len()).map(|_| ArrayData::new()).collect();
            }

            for (i, group) in group_arrays.iter_mut().enumerate() {
                let val = capture_to_val(vm, captures.get(i), offset_capture, unmatched_as_null);
                group.insert(ArrayKey::Int(match_index), val);
            }

            match_index += 1;
            count += 1;
        }

        for (i, group) in group_arrays.into_iter().enumerate() {
            let group_handle = vm.arena.alloc(Val::Array(Rc::new(group)));
            matches_array.insert(ArrayKey::Int(i as i64), group_handle);
        }
    }

    if args.len() >= 3 {
        let matches_handle = args[2];
        if vm.arena.get(matches_handle).is_ref {
            let slot = vm.arena.get_mut(matches_handle);
            slot.value = Val::Array(Rc::new(matches_array));
        }
    }

    Ok(vm.arena.alloc(Val::Int(count)))
}

pub fn preg_replace_callback(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 3 {
        return Err("preg_replace_callback expects at least 3 arguments".into());
    }

    let pattern_handle = args[0];
    let callback_handle = args[1];
    let subject_handle = args[2];

    let limit = if args.len() >= 4 {
        match vm.arena.get(args[3]).value {
            Val::Int(l) => l,
            _ => -1,
        }
    } else {
        -1
    };

    let pattern_str = match &vm.arena.get(pattern_handle).value {
        Val::String(s) => s.clone(),
        _ => return Err("preg_replace_callback pattern must be a string".into()),
    };

    let subject_str = match &vm.arena.get(subject_handle).value {
        Val::String(s) => s.clone(),
        _ => Rc::new(
            vm.convert_to_string(subject_handle)
                .map_err(|e| e.to_string())?,
        ),
    };

    let Some(regex) = compile_or_warn(vm, "preg_replace_callback", &pattern_str) else {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    };

    let mut result = Vec::new();
    let mut last_end = 0;
    let mut count = 0;

    for captures in regex.captures_iter(&subject_str) {
        let captures = captures.map_err(|e| format!("Regex error: {}", e))?;

        if let Some(m) = captures.get(0) {
            if limit != -1 && count >= limit {
                break;
            }

            result.extend_from_slice(&subject_str[last_end..m.start()]);

            let mut match_array = ArrayData::new();
            for i in 0..captures.len() {
                if let Some(mat) = captures.get(i) {
                    let match_str = mat.as_bytes().to_vec();
                    let val = vm.arena.alloc(Val::String(Rc::new(match_str)));
                    match_array.insert(ArrayKey::Int(i as i64), val);
                }
            }

            let matches_handle = vm.arena.alloc(Val::Array(Rc::new(match_array)));
            let callback_result = vm
                .call_callable(callback_handle, smallvec![matches_handle])
                .map_err(|e| e.to_string())?;
            let replacement = vm.value_to_string(callback_result)?;

            result.extend_from_slice(&replacement);
            last_end = m.end();
            count += 1;
        }
    }

    result.extend_from_slice(&subject_str[last_end..]);

    if args.len() >= 5 {
        let count_handle = args[4];
        if vm.arena.get(count_handle).is_ref {
            let slot = vm.arena.get_mut(count_handle);
            slot.value = Val::Int(count);
        }
    }

    Ok(vm.arena.alloc(Val::String(Rc::new(result))))
}

/// Expand `$n`, `${n}` and `\\n` (n = 0..99) group references.
fn interpolate_replacement(replacement: &[u8], captures: &Captures) -> Vec<u8> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < replacement.len() {
        let c = replacement[i];
        if c == b'$' || c == b'\\' {
            let braced = c == b'$' && replacement.get(i + 1) == Some(&b'{');
            let start = i + 1 + usize::from(braced);
            let len = replacement[start.min(replacement.len())..]
                .iter()
                .take(2)
                .take_while(|b| b.is_ascii_digit())
                .count();
            let closed = !braced || replacement.get(start + len) == Some(&b'}');
            if len > 0 && closed {
                let group: usize = std::str::from_utf8(&replacement[start..start + len])
                    .ok()
                    .and_then(|d| d.parse().ok())
                    .unwrap_or(0);
                if let Some(m) = captures.get(group) {
                    result.extend_from_slice(m.as_bytes());
                }
                i = start + len + usize::from(braced);
                continue;
            }
            if c == b'\\' && replacement.get(i + 1) == Some(&b'\\') {
                result.push(b'\\');
                i += 2;
                continue;
            }
        }
        result.push(c);
        i += 1;
    }
    result
}

fn capture_to_val(
    vm: &mut VM,
    capture: Option<pcre2::bytes::Match>,
    offset_capture: bool,
    unmatched_as_null: bool,
) -> Handle {
    if let Some(m) = capture {
        let match_val = vm.arena.alloc(Val::String(Rc::new(m.as_bytes().to_vec())));
        if offset_capture {
            let mut pair = ArrayData::new();
            pair.insert(ArrayKey::Int(0), match_val);
            pair.insert(ArrayKey::Int(1), vm.arena.alloc(Val::Int(m.start() as i64)));
            vm.arena.alloc(Val::Array(Rc::new(pair)))
        } else {
            match_val
        }
    } else if unmatched_as_null {
        vm.arena.alloc(Val::Null)
    } else {
        vm.arena.alloc(Val::String(Rc::new(Vec::new())))
    }
}

pub fn preg_split(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // args: pattern, subject, limit, flags
    if args.len() < 2 {
        return Err("preg_split expects at least 2 arguments".into());
    }

    let pattern_handle = args[0];
    let subject_handle = args[1];

    let pattern_str = match &vm.arena.get(pattern_handle).value {
        Val::String(s) => s.clone(),
        _ => return Err("preg_split pattern must be a string".into()),
    };

    let subject_str = match &vm.arena.get(subject_handle).value {
        Val::String(s) => s.clone(),
        _ => return Err("preg_split subject must be a string".into()),
    };

    let Some(regex) = compile_or_warn(vm, "preg_split", &pattern_str) else {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    };

    let mut result = ArrayData::new();
    let mut last_end = 0;
    let mut index = 0i64;

    for m in regex.find_iter(&subject_str) {
        let m = m.map_err(|e| format!("Regex find error: {}", e))?;
        // Add the part before the match
        let before = subject_str[last_end..m.start()].to_vec();
        let val = vm.arena.alloc(Val::String(Rc::new(before)));
        result.insert(ArrayKey::Int(index), val);
        index += 1;
        last_end = m.end();
    }

    // Add the remaining part
    let remaining = subject_str[last_end..].to_vec();
    let val = vm.arena.alloc(Val::String(Rc::new(remaining)));
    result.insert(ArrayKey::Int(index), val);

    Ok(vm.arena.alloc(Val::Array(Rc::new(result))))
}

pub fn preg_quote(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("preg_quote expects at least 1 argument".into());
    }
    let subject = vm.convert_to_string(args[0]).map_err(|e| e.to_string())?;
    let delimiter = match args.get(1) {
        Some(h) => match &vm.arena.get(*h).value {
            Val::Null => None,
            _ => vm.convert_to_string(*h).map_err(|e| e.to_string())?.first().copied(),
        },
        None => None,
    };
    Ok(vm.arena.alloc(Val::String(Rc::new(quote(&subject, delimiter)))))
}

/// Reference: $PHP_SRC_PATH/ext/pcre/php_pcre.c - preg_quote
fn quote(subject: &[u8], delimiter: Option<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(subject.len());
    for &c in subject {
        match c {
            b'.' | b'\\' | b'+' | b'*' | b'?' | b'[' | b'^' | b']' | b'$' | b'(' | b')'
            | b'{' | b'}' | b'=' | b'!' | b'>' | b'<' | b'|' | b':' | b'-' | b'#' => {
                out.push(b'\\');
                out.push(c);
            }
            0 => out.extend_from_slice(b"\\000"),
            c if Some(c) == delimiter => {
                out.push(b'\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// Compile, or emit PHP's warning (`preg_match(): Unknown modifier 'q'`)
/// and let the caller return false.
fn compile_or_warn(vm: &mut VM, func: &str, pattern: &[u8]) -> Option<Regex> {
    match compile_php_pattern(pattern) {
        Ok(regex) => Some(regex),
        Err(e) => {
            vm.trigger_error(
                crate::vm::engine::ErrorLevel::Warning,
                &format!("{}(): {}", func, e),
            );
            None
        }
    }
}

/// Compile a PHP-style delimited pattern (`/abc/i`, `{abc}x`, `#a#`) with
/// its trailing modifiers applied.
/// Reference: $PHP_SRC_PATH/ext/pcre/php_pcre.c - pcre_get_compiled_regex_cache_ex
fn compile_php_pattern(pattern: &[u8]) -> Result<Regex, String> {
    let (body, modifiers) = parse_php_pattern(pattern)?;
    let mut builder = RegexBuilder::new();
    builder.jit_if_available(true);
    let mut prefix = String::new();
    for m in modifiers.bytes() {
        match m {
            b'i' => {
                builder.caseless(true);
            }
            b'm' => {
                builder.multi_line(true);
            }
            b's' => {
                builder.dotall(true);
            }
            b'x' => {
                builder.extended(true);
            }
            b'u' => {
                builder.utf(true).ucp(true);
            }
            b'U' => prefix.push_str("(?U)"),
            b'A' => prefix.push_str("\\G"),
            b'D' | b'S' | b'X' | b'J' | b'\n' | b'\r' | b' ' => {}
            other => return Err(format!("Unknown modifier '{}'", other as char)),
        }
    }
    let source = format!("{}{}", prefix, String::from_utf8_lossy(&body));
    builder
        .build(&source)
        .map_err(|e| format!("Compilation failed: {}", e))
}

/// Split `<delim>body<delim>modifiers`; bracket delimiters close with their
/// pair and may nest, as in PHP.
fn parse_php_pattern(pattern: &[u8]) -> Result<(Vec<u8>, String), String> {
    let start = pattern
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .ok_or("Empty regular expression")?;
    let open = pattern[start];
    if open.is_ascii_alphanumeric() || open == b'\\' {
        return Err("Delimiter must not be alphanumeric, backslash, or NUL".into());
    }
    let close = match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        other => other,
    };
    let mut depth = 0usize;
    let mut i = start + 1;
    let end = loop {
        match pattern.get(i) {
            None => {
                return Err(format!("No ending delimiter '{}' found", close as char));
            }
            Some(b'\\') => i += 1,
            Some(&c) if c == close && depth == 0 => break i,
            Some(&c) if c == close => depth -= 1,
            Some(&c) if c == open && open != close => depth += 1,
            _ => {}
        }
        i += 1;
    };
    let body = pattern[start + 1..end].to_vec();
    let modifiers = String::from_utf8_lossy(&pattern[end + 1..]).into_owned();
    Ok((body, modifiers))
}
