use indexmap::IndexMap;
use std::any::Any;
use std::collections::HashSet;
use std::fmt::Debug;
use std::rc::Rc;

/// Array metadata for efficient operations
/// Reference: $PHP_SRC_PATH/Zend/zend_hash.h - HashTable::nNextFreeElement
#[derive(Debug, Clone)]
pub struct ArrayData {
    pub map: IndexMap<ArrayKey, Handle>,
    pub next_free: i64, // Cached next auto-increment index (like HashTable::nNextFreeElement)
}

impl ArrayData {
    pub fn new() -> Self {
        Self {
            map: IndexMap::new(),
            next_free: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            map: IndexMap::with_capacity(capacity),
            next_free: 0,
        }
    }

    /// Insert a key-value pair and update next_free if needed
    /// Reference: $PHP_SRC_PATH/Zend/zend_hash.c - _zend_hash_index_add_or_update_i
    pub fn insert(&mut self, key: ArrayKey, value: Handle) -> Option<Handle> {
        if let ArrayKey::Int(i) = &key {
            if *i >= self.next_free {
                self.next_free = i + 1;
            }
        }
        self.map.insert(key, value)
    }

    /// Get the next auto-increment index (O(1))
    /// Reference: $PHP_SRC_PATH/Zend/zend_hash.c - zend_hash_next_free_element
    pub fn next_index(&self) -> i64 {
        self.next_free
    }

    /// Append a value with auto-incremented key
    pub fn push(&mut self, value: Handle) {
        let key = ArrayKey::Int(self.next_free);
        self.next_free += 1;
        self.map.insert(key, value);
    }
}

impl From<IndexMap<ArrayKey, Handle>> for ArrayData {
    fn from(map: IndexMap<ArrayKey, Handle>) -> Self {
        // Compute next_free from existing keys
        let next_free = map
            .keys()
            .filter_map(|k| match k {
                ArrayKey::Int(i) => Some(*i),
                ArrayKey::Str(s) => {
                    // PHP also considers numeric string keys
                    if let Ok(s_str) = std::str::from_utf8(s) {
                        s_str.parse::<i64>().ok()
                    } else {
                        None
                    }
                }
            })
            .max()
            .map(|i| i + 1)
            .unwrap_or(0);

        Self { map, next_free }
    }
}

impl PartialEq for ArrayData {
    fn eq(&self, other: &Self) -> bool {
        self.map == other.map
        // Don't compare next_free as it's cached metadata
    }
}

/// Reference to a heap slot. The generation distinguishes successive values
/// stored in the same slot, so a handle that outlived its value is detected
/// instead of silently reading whatever reused the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle {
    index: u32,
    generation: u32,
}

impl Handle {
    pub(crate) fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// Slot index (stable for the value's lifetime; used for object ids).
    pub fn index(self) -> u32 {
        self.index
    }

    pub(crate) fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Symbol(pub u32); // Interned String

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Protected,
    Private,
}

/// Key type for compile-time constant arrays
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstArrayKey {
    Int(i64),
    Str(Rc<Vec<u8>>),
}

#[derive(Debug, Clone)]
pub enum Val {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Rc<Vec<u8>>),  // PHP strings are byte arrays (COW)
    Array(Rc<ArrayData>), // Array with cached metadata (COW)
    ConstArray(Rc<IndexMap<ConstArrayKey, Val>>), // Compile-time constant array (template for property defaults)
    Object(Handle),
    ObjPayload(ObjectData),
    Resource(Rc<dyn Any>), // Changed to Rc to support Clone
    AppendPlaceholder,     // Internal use for $a[]
    Uninitialized,
}

impl PartialEq for Val {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Val::Null, Val::Null) => true,
            (Val::Bool(a), Val::Bool(b)) => a == b,
            (Val::Int(a), Val::Int(b)) => a == b,
            (Val::Float(a), Val::Float(b)) => a == b,
            (Val::String(a), Val::String(b)) => a == b,
            (Val::Array(a), Val::Array(b)) => a == b,
            (Val::ConstArray(a), Val::ConstArray(b)) => a == b,
            (Val::Object(a), Val::Object(b)) => a == b,
            (Val::ObjPayload(a), Val::ObjPayload(b)) => a == b,
            (Val::Resource(a), Val::Resource(b)) => Rc::ptr_eq(a, b),
            (Val::AppendPlaceholder, Val::AppendPlaceholder) => true,
            (Val::Uninitialized, Val::Uninitialized) => true,
            _ => false,
        }
    }
}

/// A float as an int, as zend_dval_to_lval does on 64-bit: NAN and the
/// infinities are 0, and values out of range wrap modulo 2^64.
pub fn dval_to_lval(d: f64) -> i64 {
    if !d.is_finite() {
        return 0;
    }
    if (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&d) {
        return d as i64;
    }
    let two_pow_64 = 18_446_744_073_709_551_616.0_f64;
    let mut dmod = d % two_pow_64;
    if dmod < 0.0 {
        // -2^63 stays itself; anything else becomes positive first
        if dmod == -9.223_372_036_854_776e18 {
            return i64::MIN;
        }
        dmod += two_pow_64;
    }
    if dmod > 9.223_372_036_854_775_807e18 {
        dmod -= two_pow_64;
    }
    dmod as i64
}

/// How many significant digits a float is printed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatPrecision {
    /// PHP's `precision` ini (14): echo, string casts, print_r.
    Digits(usize),
    /// PHP's `serialize_precision = -1`: shortest round-trip digits; var_dump,
    /// var_export, json_encode and serialize.
    Shortest,
}

thread_local! {
    /// The `precision` and `serialize_precision` ini values of the request
    /// running on this thread.
    static PRECISION_INI: std::cell::Cell<(i64, i64)> = const { std::cell::Cell::new((14, -1)) };
}

impl FloatPrecision {
    /// The `precision` ini default.
    pub const ECHO: FloatPrecision = FloatPrecision::Digits(14);

    /// From an ini value: -1 means the shortest round-trip form, 0 one digit.
    fn from_ini(value: i64) -> FloatPrecision {
        if value < 0 {
            FloatPrecision::Shortest
        } else {
            FloatPrecision::Digits((value as usize).clamp(1, 40))
        }
    }

    /// The current `precision` ini: echo, string casts, print_r.
    pub fn echo() -> FloatPrecision {
        Self::from_ini(PRECISION_INI.with(|p| p.get().0))
    }

    /// The current `serialize_precision` ini: var_dump, var_export,
    /// json_encode, serialize.
    pub fn serialize() -> FloatPrecision {
        Self::from_ini(PRECISION_INI.with(|p| p.get().1))
    }

    /// Record a change of `precision` or `serialize_precision` (other names
    /// are ignored); the value is read as PHP reads an int ini.
    pub fn set_ini(name: &str, value: &[u8]) {
        let parsed = Val::String(Rc::new(value.to_vec())).to_int();
        PRECISION_INI.with(|p| {
            let (precision, serialize) = p.get();
            match name {
                "precision" => p.set((parsed, serialize)),
                "serialize_precision" => p.set((precision, parsed)),
                _ => {}
            }
        });
    }
}

/// Format a float the way zend_gcvt does: `precision` significant digits,
/// exponent form (`1.0E+25`) when the decimal point falls outside the digits.
/// Reference: $PHP_SRC_PATH/main/snprintf.c - php_gcvt
pub fn php_float_to_string(f: f64, precision: FloatPrecision, exp_char: char) -> String {
    if f.is_nan() {
        return "NAN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "INF" } else { "-INF" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0" } else { "0" }.into();
    }
    let (sci, ndigit) = match precision {
        FloatPrecision::Shortest => (format!("{:e}", f.abs()), 17),
        FloatPrecision::Digits(p) => {
            let p = p.max(1);
            (format!("{:.*e}", p - 1, f.abs()), p as i64)
        }
    };
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let mut digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let decpt = exp.parse::<i64>().unwrap_or(0) + 1;
    let mut out = String::new();
    if f < 0.0 {
        out.push('-');
    }
    if decpt < -3 || decpt > ndigit {
        out.push_str(&digits[..1]);
        out.push('.');
        out.push_str(if digits.len() > 1 { &digits[1..] } else { "0" });
        out.push(exp_char);
        out.push(if decpt - 1 < 0 { '-' } else { '+' });
        out.push_str(&(decpt - 1).abs().to_string());
    } else if decpt <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-decpt) as usize));
        out.push_str(&digits);
    } else if digits.len() as i64 <= decpt {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', decpt as usize - digits.len()));
    } else {
        out.push_str(&digits[..decpt as usize]);
        out.push('.');
        out.push_str(&digits[decpt as usize..]);
    }
    out
}

impl Val {
    pub fn type_name(&self) -> &'static str {
        match self {
            Val::Null => "null",
            Val::Bool(_) => "bool",
            Val::Int(_) => "int",
            Val::Float(_) => "float",
            Val::String(_) => "string",
            Val::Array(_) | Val::ConstArray(_) => "array",
            Val::Object(_) | Val::ObjPayload(_) => "object",
            Val::Resource(_) => "resource",
            Val::AppendPlaceholder => "append_placeholder",
            Val::Uninitialized => "uninitialized",
        }
    }

    /// Convert to string following PHP's zend_make_printable_zval semantics
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - zend_make_printable_zval
    pub fn to_php_string_bytes(&self) -> Vec<u8> {
        match self {
            Val::Null => Vec::new(),
            Val::Bool(b) => {
                if *b {
                    b"1".to_vec()
                } else {
                    Vec::new()
                }
            }
            Val::Int(i) => i.to_string().into_bytes(),
            Val::Float(f) => php_float_to_string(*f, FloatPrecision::echo(), 'E').into_bytes(),
            Val::String(s) => s.to_vec(),
            Val::Array(_) | Val::ConstArray(_) => b"Array".to_vec(),
            Val::Object(_) | Val::ObjPayload(_) => b"Object".to_vec(),
            Val::Resource(_) => b"Resource".to_vec(),
            Val::AppendPlaceholder | Val::Uninitialized => Vec::new(),
        }
    }

    /// Convert to boolean following PHP's zend_is_true semantics
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - zend_is_true
    pub fn to_bool(&self) -> bool {
        match self {
            Val::Null => false,
            Val::Bool(b) => *b,
            Val::Int(i) => *i != 0,
            Val::Float(f) => *f != 0.0,
            Val::String(s) => {
                // Empty string or "0" is false
                if s.is_empty() {
                    false
                } else if s.len() == 1 && s[0] == b'0' {
                    false
                } else {
                    true
                }
            }
            Val::Array(arr) => !arr.map.is_empty(),
            Val::ConstArray(arr) => !arr.is_empty(),
            Val::Object(_) | Val::ObjPayload(_) | Val::Resource(_) => true,
            Val::AppendPlaceholder | Val::Uninitialized => false,
        }
    }

    /// Convert to integer following PHP's convert_to_long semantics
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - convert_to_long
    pub fn to_int(&self) -> i64 {
        match self {
            Val::Null => 0,
            Val::Bool(b) => {
                if *b {
                    1
                } else {
                    0
                }
            }
            Val::Int(i) => *i,
            Val::Float(f) => dval_to_lval(*f),
            Val::String(s) => {
                // Parse numeric string
                Self::parse_numeric_string(s)
            }
            Val::Array(arr) => {
                if arr.map.is_empty() {
                    0
                } else {
                    1
                }
            }
            Val::ConstArray(arr) => {
                if arr.is_empty() {
                    0
                } else {
                    1
                }
            }
            Val::Object(_) | Val::ObjPayload(_) => 1,
            Val::Resource(_) => 0, // Resources typically convert to their ID
            Val::AppendPlaceholder | Val::Uninitialized => 0,
        }
    }

    /// Convert to float following PHP's convert_to_double semantics
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - convert_to_double
    pub fn to_float(&self) -> f64 {
        match self {
            Val::Null => 0.0,
            Val::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Val::Int(i) => *i as f64,
            Val::Float(f) => *f,
            Val::String(s) => {
                // Parse numeric string
                std::str::from_utf8(Self::numeric_prefix(s))
                    .ok()
                    .and_then(|t| t.parse::<f64>().ok())
                    .unwrap_or(0.0)
            }
            Val::Array(arr) => {
                if arr.map.is_empty() {
                    0.0
                } else {
                    1.0
                }
            }
            Val::ConstArray(arr) => {
                if arr.is_empty() {
                    0.0
                } else {
                    1.0
                }
            }
            Val::Object(_) | Val::ObjPayload(_) => 1.0,
            Val::Resource(_) => 0.0,
            Val::AppendPlaceholder | Val::Uninitialized => 0.0,
        }
    }

    /// Parse numeric string to int, returning (value, is_float)
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_numeric_string_ex
    /// Leading-numeric parse as PHP's casts do it ("12abc" -> 12, " 1.5e3x" -> 1500):
    /// whitespace, sign, digits, optional fraction and exponent; the rest is ignored.
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - _zend_handle_numeric_str_ex
    fn parse_numeric_string(s: &[u8]) -> i64 {
        let Ok(text) = std::str::from_utf8(Self::numeric_prefix(s)) else {
            return 0;
        };
        let is_float = text.contains(['.', 'e', 'E']);
        if !is_float && let Ok(i) = text.parse::<i64>() {
            return i;
        }
        text.parse::<f64>().map_or(0, |f| f as i64)
    }

    /// The value of `s` if the whole string is numeric (leading and trailing
    /// whitespace allowed, as in PHP 8), as an Int when it fits, else a Float.
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_numeric_string_ex
    pub fn numeric_string(s: &[u8]) -> Option<Val> {
        let (value, rest) = Self::leading_numeric(s)?;
        rest.iter()
            .all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c'))
            .then_some(value)
    }

    /// The value of the numeric prefix of `s` ("12abc" -> 12) and the bytes
    /// after it, or None when `s` does not start with a number.
    pub fn leading_numeric(s: &[u8]) -> Option<(Val, &[u8])> {
        let prefix = Self::numeric_prefix(s);
        if prefix.is_empty() {
            return None;
        }
        let end = prefix.as_ptr() as usize - s.as_ptr() as usize + prefix.len();
        let text = std::str::from_utf8(prefix).ok()?;
        let value = match text.parse::<i64>() {
            Ok(i) if !text.contains(['.', 'e', 'E']) => Val::Int(i),
            _ => Val::Float(text.parse::<f64>().ok()?),
        };
        Some((value, &s[end..]))
    }

    /// The numeric prefix of `s` after leading whitespace (may be empty).
    fn numeric_prefix(s: &[u8]) -> &[u8] {
        let start = s
            .iter()
            .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c'))
            .unwrap_or(s.len());
        let s = &s[start..];
        let digits = |from: usize| s[from..].iter().take_while(|b| b.is_ascii_digit()).count();
        let mut end = usize::from(matches!(s.first(), Some(b'+' | b'-')));
        let int_digits = digits(end);
        end += int_digits;
        let mut frac_digits = 0;
        if s.get(end) == Some(&b'.') {
            frac_digits = digits(end + 1);
            if int_digits + frac_digits > 0 {
                end += 1 + frac_digits;
            }
        }
        if int_digits + frac_digits == 0 {
            return &s[..0];
        }
        if matches!(s.get(end), Some(b'e' | b'E')) {
            let sign = usize::from(matches!(s.get(end + 1), Some(b'+' | b'-')));
            let exp_digits = digits(end + 1 + sign);
            if exp_digits > 0 {
                end += 1 + sign + exp_digits;
            }
        }
        &s[..end]
    }
}

#[derive(Debug, Clone)]
pub struct ObjectData {
    // Placeholder for object data
    pub class: Symbol,
    pub properties: IndexMap<Symbol, Handle>,
    pub internal: Option<Rc<dyn Any>>, // For internal classes like Closure
    pub dynamic_properties: HashSet<Symbol>, // Track which properties are dynamic (created at runtime)
    /// The object's id (var_dump's #N, spl_object_id), numbered from 1 in
    /// creation order; set by the heap when the object is allocated.
    pub id: u32,
}

impl PartialEq for ObjectData {
    fn eq(&self, other: &Self) -> bool {
        self.class == other.class && self.properties == other.properties
        // Ignore internal for equality for now, or check ptr_eq
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum ArrayKey {
    Int(i64),
    Str(Rc<Vec<u8>>),
}

// The Container (Zval equivalent)
#[derive(Debug, Clone)]
pub struct Zval {
    pub value: Val,
    pub is_ref: bool, // Explicit Reference Flag (&$a)
}

#[cfg(test)]
mod float_format_tests {
    use super::{FloatPrecision, php_float_to_string};

    // Expected strings are PHP 8.4's output for the same values.
    #[test]
    fn shortest_matches_var_dump() {
        let f = |v| php_float_to_string(v, FloatPrecision::Shortest, 'E');
        assert_eq!(f(1.0), "1");
        assert_eq!(f(-0.0), "-0");
        assert_eq!(f(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(f(1e16), "10000000000000000");
        assert_eq!(f(1.5e17), "1.5E+17");
        assert_eq!(f(1e25), "1.0E+25");
        assert_eq!(f(0.0001), "0.0001");
        assert_eq!(f(1e-5), "1.0E-5");
        assert_eq!(f(-2.5e-5), "-2.5E-5");
        assert_eq!(f(123.456), "123.456");
    }

    #[test]
    fn precision_14_matches_echo() {
        let f = |v| php_float_to_string(v, FloatPrecision::ECHO, 'E');
        assert_eq!(f(0.1 + 0.2), "0.3");
        assert_eq!(f(1.0 / 3.0), "0.33333333333333");
        assert_eq!(f(1e14), "1.0E+14");
        assert_eq!(f(123456789012345.0), "1.2345678901234E+14");
        assert_eq!(f(1234567890123456.0), "1.2345678901235E+15");
        assert_eq!(f(9007199254740993.0), "9.007199254741E+15");
        assert_eq!(f(-1.5), "-1.5");
        assert_eq!(f(100.0), "100");
    }
}
