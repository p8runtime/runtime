//! php.ini directives as a request sees them: ini_get(), ini_set(),
//! ini_restore(). Values are PHP 8.4's built-in defaults for the CLI with no
//! php.ini; a directive not listed here does not exist (ini_get() returns
//! false, like PHP for an unknown name).
//!
//! `error_reporting` is kept in PhpConfig (the engine reads it constantly);
//! the builtins bridge it.

use std::collections::HashMap;

/// (name, default) for the directives this runtime knows.
const DEFAULTS: &[(&str, &str)] = &[
    ("display_errors", "1"),
    ("display_startup_errors", "1"),
    ("log_errors", "0"),
    ("html_errors", "0"),
    ("error_log", ""),
    ("error_prepend_string", ""),
    ("error_append_string", ""),
    ("ignore_repeated_errors", "0"),
    ("track_errors", "0"),
    ("precision", "14"),
    ("serialize_precision", "-1"),
    ("memory_limit", "128M"),
    ("max_execution_time", "0"),
    ("default_charset", "UTF-8"),
    ("date.timezone", "UTC"),
    ("output_buffering", "0"),
    ("implicit_flush", "1"),
    ("short_open_tag", "1"),
    ("zend.assertions", "-1"),
    ("assert.active", "1"),
    ("assert.exception", "1"),
    ("assert.warning", "1"),
    ("assert.bail", "0"),
    ("assert.callback", ""),
    ("include_path", ".:/usr/local/lib/php"),
    ("auto_detect_line_endings", "0"),
    ("default_socket_timeout", "60"),
    ("user_agent", ""),
    ("allow_url_fopen", "1"),
    ("arg_separator.output", "&"),
    ("arg_separator.input", "&"),
    ("variables_order", "EGPCS"),
    ("request_order", ""),
    ("register_argc_argv", "1"),
    ("upload_max_filesize", "2M"),
    ("post_max_size", "8M"),
    ("max_input_time", "-1"),
    ("max_input_vars", "1000"),
    ("pcre.backtrack_limit", "1000000"),
    ("pcre.recursion_limit", "100000"),
    ("pcre.jit", "1"),
    ("session.save_handler", "files"),
    ("session.name", "PHPSESSID"),
    ("mbstring.language", "neutral"),
    ("zend.exception_ignore_args", "0"),
    ("zend.exception_string_param_max_len", "15"),
    ("unserialize_callback_func", ""),
    ("unserialize_max_depth", "4096"),
];

#[derive(Debug, Clone, Default)]
pub struct IniSettings {
    changed: HashMap<String, Vec<u8>>,
}

impl IniSettings {
    fn default_of(name: &str) -> Option<&'static str> {
        DEFAULTS.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }

    /// The current value, or None for an unknown directive.
    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        if let Some(v) = self.changed.get(name) {
            return Some(v.clone());
        }
        Self::default_of(name).map(|v| v.as_bytes().to_vec())
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
