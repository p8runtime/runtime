use regex::Regex;

#[derive(Debug, Clone)]
pub enum ExpectationType {
    Exact(String),
    Format(String),
    Regex(String),
}

/// Match actual output against expected output
pub fn match_output(actual: &str, expected: ExpectationType) -> bool {
    match expected {
        ExpectationType::Exact(expected_str) => {
            normalize_output(actual) == normalize_output(&expected_str)
        }
        ExpectationType::Format(format_str) => {
            match_expectf(actual, &format_str)
        }
        ExpectationType::Regex(regex_str) => {
            match_regex(actual, &regex_str)
        }
    }
}

/// Normalize like run-tests.php: CRLF -> LF, surrounding whitespace trimmed.
fn normalize_output(s: &str) -> String {
    s.replace("\r\n", "\n").trim().to_string()
}

/// Match output against EXPECTF format (with %s, %d, %i, %f, etc.)
fn match_expectf(actual: &str, format_str: &str) -> bool {
    let regex_pattern = expectf_to_regex(&normalize_output(format_str));
    match Regex::new(&regex_pattern) {
        Ok(re) => re.is_match(&normalize_output(actual)),
        Err(_) => false,
    }
}

/// Convert EXPECTF format to a regex, with run-tests.php's placeholder set.
/// Reference: $PHP_SRC_PATH/run-tests.php - expectf_to_regex
fn expectf_to_regex(format_str: &str) -> String {
    let mut pattern = String::from("(?s)^");
    let mut rest = format_str;
    while !rest.is_empty() {
        // %r...%r embeds raw regex
        if let Some(after) = rest.strip_prefix("%r")
            && let Some(end) = after.find("%r")
        {
            pattern.push('(');
            pattern.push_str(&after[..end]);
            pattern.push(')');
            rest = &after[end + 2..];
            continue;
        }
        let mut chars = rest.chars();
        let ch = chars.next().unwrap();
        if ch == '%'
            && let Some(code) = chars.next()
        {
            let piece = match code {
                'e' => Some(r"[/\\]"),
                's' => Some(r"[^\r\n]+"),
                'S' => Some(r"[^\r\n]*"),
                'a' => Some(r".+"),
                'A' => Some(r".*"),
                'w' => Some(r"\s*"),
                'i' => Some(r"[+-]?\d+"),
                'd' => Some(r"\d+"),
                'x' => Some(r"[0-9a-fA-F]+"),
                'f' => Some(r"[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[Ee][+-]?\d+)?"),
                'c' => Some(r"."),
                '0' => Some(r"\x00"),
                _ => None,
            };
            if let Some(piece) = piece {
                pattern.push_str(piece);
                rest = &rest[2..];
                continue;
            }
        }
        pattern.push_str(&regex::escape(&ch.to_string()));
        rest = &rest[ch.len_utf8()..];
    }
    pattern.push('$');
    pattern
}

/// EXPECTREGEX is anchored and dot-matches-newline, as run-tests.php uses it.
fn match_regex(actual: &str, regex_str: &str) -> bool {
    match Regex::new(&format!("(?s)^(?:{})$", normalize_output(regex_str))) {
        Ok(re) => re.is_match(&normalize_output(actual)),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert!(match_output("Hello World", ExpectationType::Exact("Hello World".to_string())));
        assert!(match_output("Hello World\n", ExpectationType::Exact("Hello World".to_string())));
        assert!(!match_output("Hello", ExpectationType::Exact("Hello World".to_string())));
    }

    #[test]
    fn test_expectf_string() {
        let format = "Error: %s at line %d".to_string();
        assert!(match_output(
            "Error: file not found at line 42",
            ExpectationType::Format(format.clone())
        ));
        assert!(!match_output(
            "Error: 123 at line abc",
            ExpectationType::Format(format)
        ));
    }

    #[test]
    fn test_expectf_integer() {
        let format = "Value: %d".to_string();
        assert!(match_output("Value: 42", ExpectationType::Format(format.clone())));
        assert!(!match_output("Value: -42", ExpectationType::Format(format.clone())));
        assert!(match_output("Value: -42", ExpectationType::Format("Value: %i".into())));
        assert!(!match_output("Value: abc", ExpectationType::Format(format)));
    }

    #[test]
    fn test_expectf_float() {
        let format = "Pi: %f".to_string();
        assert!(match_output("Pi: 3.14159", ExpectationType::Format(format.clone())));
        assert!(match_output("Pi: 3", ExpectationType::Format(format.clone())));
    }

    #[test]
    fn test_expectf_php_src_semantics() {
        // %s is one or more non-newline chars; %a spans lines; %d is unsigned
        assert!(!match_output("xy", ExpectationType::Format("x%sy".into())));
        assert!(match_output("xy", ExpectationType::Format("x%Sy".into())));
        assert!(!match_output("a\nb", ExpectationType::Format("%s".into())));
        assert!(match_output("a\nb", ExpectationType::Format("%a".into())));
        assert!(!match_output("-1", ExpectationType::Format("%d".into())));
        assert!(match_output("-1", ExpectationType::Format("%i".into())));
        assert!(match_output("1.5E+25", ExpectationType::Format("%f".into())));
        assert!(match_output(
            "Warning: foo in /x/y.php on line 3",
            ExpectationType::Format("Warning: foo in %s on line %d".into())
        ));
        assert!(match_output("v1.2", ExpectationType::Format("v%r\\d\\.\\d%r".into())));
    }

    #[test]
    fn test_regex_match() {
        let regex = r"^\d{4}-\d{2}-\d{2}$".to_string();
        assert!(match_output("2024-01-15", ExpectationType::Regex(regex.clone())));
        assert!(!match_output("2024-1-15", ExpectationType::Regex(regex)));
    }
}
