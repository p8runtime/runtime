//! php.ini directives as a request sees them: ini_get(), ini_set(),
//! ini_restore(). Defaults are PHP 8.4's built-in ones with no php.ini
//! (generated, `builtins::defaults`); a directive no loaded extension
//! declares does not exist (ini_get() returns false, like PHP).
//!
//! `error_reporting` is kept in PhpConfig (the engine reads it constantly);
//! the builtins bridge it.

use crate::builtins::defaults::INI_DEFAULTS;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct IniSettings {
    changed: HashMap<String, Vec<u8>>,
    /// Loaded extensions, lowercase: their directives exist.
    extensions: Vec<String>,
}

impl IniSettings {
    /// The directives of the engine and of the `loaded` extensions.
    pub fn new(loaded: &[&str]) -> Self {
        Self {
            changed: HashMap::new(),
            extensions: loaded.iter().map(|name| name.to_ascii_lowercase()).collect(),
        }
    }

    /// The built-in default, or None if the directive does not exist.
    fn default_of(&self, name: &str) -> Option<&'static [u8]> {
        INI_DEFAULTS
            .iter()
            .find(|(ext, n, _)| {
                *n == name && (*ext == "core" || self.extensions.iter().any(|e| e == ext))
            })
            .map(|(_, _, value)| value.unwrap_or(b""))
    }

    /// The current value, or None for an unknown directive.
    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        if let Some(v) = self.changed.get(name) {
            return Some(v.clone());
        }
        self.default_of(name).map(<[u8]>::to_vec)
    }

    /// Set a directive; returns the old value, or None if it does not exist.
    pub fn set(&mut self, name: &str, value: Vec<u8>) -> Option<Vec<u8>> {
        let old = self.get(name)?;
        self.changed.insert(name.to_string(), value);
        Some(old)
    }

    /// Back to the default.
    pub fn restore(&mut self, name: &str) {
        self.changed.remove(name);
    }

    /// Whether a boolean directive is on ("1", "On", "yes", "true", "stdout", ...).
    pub fn is_on(&self, name: &str) -> bool {
        match self.get(name) {
            Some(v) => {
                let v = String::from_utf8_lossy(&v).to_ascii_lowercase();
                !matches!(v.trim(), "" | "0" | "off" | "no" | "false" | "none")
            }
            None => false,
        }
    }
}

/// A value as php.ini and `-d` write it, read the way PHP's ini scanner
/// reads it: quotes are removed, `On`/`yes`/`true` become "1" and
/// `Off`/`no`/`false`/`none`/`null` become "", and an expression of
/// constants and integers with `| & ^ ~ !` and parentheses is evaluated
/// (`E_ALL & ~E_DEPRECATED`). `constant` looks a constant's value up.
pub fn parse_value(raw: &str, constant: impl Fn(&str) -> Option<Vec<u8>>) -> Vec<u8> {
    let raw = raw.trim();
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        return raw.as_bytes()[1..raw.len() - 1].to_vec();
    }
    match raw.to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" => return b"1".to_vec(),
        "off" | "no" | "false" | "none" | "null" => return Vec::new(),
        _ => {}
    }
    let is_name = |s: &str| {
        s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    if is_name(raw) {
        return constant(raw).unwrap_or_else(|| raw.as_bytes().to_vec());
    }
    if raw.contains(['|', '&', '^', '~', '!', '(']) {
        let mut parser = Expr { tokens: tokenize(raw), pos: 0, constant: &constant };
        if let Some(value) = parser.or()
            && parser.pos == parser.tokens.len()
        {
            return value.to_string().into_bytes();
        }
    }
    raw.as_bytes().to_vec()
}

fn tokenize(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = raw.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if !(c.is_ascii_alphanumeric() || c == '_' || (c == '-' && word.is_empty())) {
                    break;
                }
                word.push(c);
                chars.next();
            }
            tokens.push(word);
        } else {
            tokens.push(c.to_string());
            chars.next();
        }
    }
    tokens
}

/// The ini scanner's expression grammar: `|` binds loosest, then `^`, `&`,
/// then the prefix operators `~` and `!`.
struct Expr<'a, F> {
    tokens: Vec<String>,
    pos: usize,
    constant: &'a F,
}

impl<F: Fn(&str) -> Option<Vec<u8>>> Expr<'_, F> {
    fn eat(&mut self, op: &str) -> bool {
        let found = self.tokens.get(self.pos).is_some_and(|t| t == op);
        if found {
            self.pos += 1;
        }
        found
    }

    fn or(&mut self) -> Option<i64> {
        let mut value = self.xor()?;
        while self.eat("|") {
            value |= self.xor()?;
        }
        Some(value)
    }

    fn xor(&mut self) -> Option<i64> {
        let mut value = self.and()?;
        while self.eat("^") {
            value ^= self.and()?;
        }
        Some(value)
    }

    fn and(&mut self) -> Option<i64> {
        let mut value = self.unary()?;
        while self.eat("&") {
            value &= self.unary()?;
        }
        Some(value)
    }

    fn unary(&mut self) -> Option<i64> {
        if self.eat("~") {
            return Some(!self.unary()?);
        }
        if self.eat("!") {
            return Some((self.unary()? == 0) as i64);
        }
        if self.eat("(") {
            let value = self.or()?;
            return self.eat(")").then_some(value);
        }
        let token = self.tokens.get(self.pos)?.clone();
        self.pos += 1;
        if let Ok(number) = token.parse::<i64>() {
            return Some(number);
        }
        let value = (self.constant)(&token)?;
        String::from_utf8_lossy(&value).trim().parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::parse_value;

    fn constant(name: &str) -> Option<Vec<u8>> {
        match name {
            "E_ALL" => Some(b"32767".to_vec()),
            "E_DEPRECATED" => Some(b"8192".to_vec()),
            "E_NOTICE" => Some(b"8".to_vec()),
            _ => None,
        }
    }

    #[test]
    fn values_read_like_php_ini() {
        assert_eq!(parse_value("On", constant), b"1");
        assert_eq!(parse_value("off", constant), b"");
        assert_eq!(parse_value("\"a b\"", constant), b"a b");
        assert_eq!(parse_value("E_ALL", constant), b"32767");
        assert_eq!(parse_value("E_ALL & ~E_DEPRECATED", constant), b"24575");
        assert_eq!(parse_value("E_ALL ^ (E_NOTICE | E_DEPRECATED)", constant), b"24567");
        assert_eq!(parse_value("128M", constant), b"128M");
        assert_eq!(parse_value("-1", constant), b"-1");
        assert_eq!(parse_value("UTC", constant), b"UTC");
    }
}
