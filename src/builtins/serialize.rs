//! serialize() and unserialize().
//!
//! Reference: $PHP_SRC_PATH/ext/standard/var.c (php_var_serialize) and
//! $PHP_SRC_PATH/ext/standard/var_unserializer.re (php_var_unserialize).
//!
//! Values are numbered in the order they are written (PHP's var_hash), so a
//! second occurrence of an object is written as `r:N;` and of a PHP reference
//! as `R:N;`; unserialize rebuilds the same sharing from those numbers.

use crate::core::value::{ArrayData, ArrayKey, FloatPrecision, Handle, ObjectData, Symbol, Val, Visibility};
use crate::core::value::php_float_to_string;
use crate::runtime::context::ClassDef;
use crate::vm::engine::{ErrorLevel, PropertyCollectionMode, VM, VmError};
use std::collections::HashMap;
use std::rc::Rc;

const INCOMPLETE_CLASS: &[u8] = b"__PHP_Incomplete_Class";
const INCOMPLETE_CLASS_NAME: &[u8] = b"__PHP_Incomplete_Class_Name";

pub fn php_serialize(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let Some(&value) = args.first() else {
        return Err("serialize() expects exactly 1 argument, 0 given".into());
    };
    let mut ser = Serializer::default();
    ser.value(vm, value)?;
    Ok(vm.arena.alloc(Val::String(Rc::new(ser.out))))
}

pub fn php_unserialize(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let Some(&data) = args.first() else {
        return Err("unserialize() expects at least 1 argument, 0 given".into());
    };
    let data = match &vm.arena.get(data).value {
        Val::String(s) => s.clone(),
        other => Rc::new(other.to_php_string_bytes()),
    };
    let options = Options::read(vm, args.get(1).copied())?;

    let mut un = Unserializer {
        data: &data,
        pos: 0,
        vars: Vec::new(),
        deferred: Vec::new(),
        options,
        depth: 0,
    };
    let result = un.value(vm, false);
    match result {
        Ok(handle) => {
            if un.pos < data.len() {
                vm.report_error(
                    ErrorLevel::Warning,
                    &format!(
                        "unserialize(): Extra data starting at offset {} of {} bytes",
                        un.pos,
                        data.len()
                    ),
                );
            }
            un.run_deferred(vm)?;
            Ok(handle)
        }
        Err(Failure::Php(e)) => Err(e),
        Err(Failure::Syntax { offset, eof }) => {
            if eof {
                vm.report_error(ErrorLevel::Warning, "unserialize(): Unexpected end of serialized data");
            }
            vm.report_error(
                ErrorLevel::Warning,
                &format!("unserialize(): Error at offset {} of {} bytes", offset, data.len()),
            );
            Ok(vm.arena.alloc(Val::Bool(false)))
        }
    }
}

// ---------------------------------------------------------------------------
// serialize
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Serializer {
    out: Vec<u8>,
    /// Number of values written so far (PHP's var_hash `n`).
    n: i64,
    /// Objects (by payload) and references (by slot) already written, with their number.
    seen: HashMap<Handle, i64>,
}

impl Serializer {
    fn string(&mut self, s: &[u8]) {
        self.out.extend_from_slice(format!("s:{}:\"", s.len()).as_bytes());
        self.out.extend_from_slice(s);
        self.out.extend_from_slice(b"\";");
    }

    fn key(&mut self, key: &ArrayKey) {
        match key {
            ArrayKey::Int(i) => self.out.extend_from_slice(format!("i:{};", i).as_bytes()),
            ArrayKey::Str(s) => self.string(s),
        }
    }

    /// Register a value in the var hash; returns the number of an earlier
    /// occurrence when this object or reference was already written.
    fn already_written(&mut self, vm: &VM, handle: Handle) -> Option<(i64, bool)> {
        self.n += 1;
        let slot = vm.arena.get(handle);
        let key = match &slot.value {
            Val::Object(payload) => *payload,
            _ if slot.is_ref => handle,
            _ => return None,
        };
        let is_ref = slot.is_ref && !matches!(slot.value, Val::Object(_));
        match self.seen.get(&key) {
            Some(&n) => {
                // A reference occupies a single number.
                if is_ref {
                    self.n -= 1;
                }
                Some((n, is_ref))
            }
            None => {
                self.seen.insert(key, self.n);
                None
            }
        }
    }

    fn value(&mut self, vm: &mut VM, handle: Handle) -> Result<(), String> {
        if let Some((n, is_ref)) = self.already_written(vm, handle) {
            let tag = if is_ref { 'R' } else { 'r' };
            self.out.extend_from_slice(format!("{}:{};", tag, n).as_bytes());
            return Ok(());
        }
        let val = vm.arena.get(handle).value.clone();
        match val {
            Val::Null | Val::Uninitialized | Val::AppendPlaceholder => self.out.extend_from_slice(b"N;"),
            Val::Bool(b) => self.out.extend_from_slice(if b { b"b:1;" } else { b"b:0;" }),
            Val::Int(i) => self.out.extend_from_slice(format!("i:{};", i).as_bytes()),
            Val::Float(f) => {
                self.out.extend_from_slice(b"d:");
                self.out
                    .extend_from_slice(php_float_to_string(f, FloatPrecision::Shortest, 'E').as_bytes());
                self.out.push(b';');
            }
            Val::String(s) => self.string(&s),
            Val::Resource(_) => self.out.extend_from_slice(b"i:0;"),
            Val::Array(arr) => {
                self.out
                    .extend_from_slice(format!("a:{}:{{", arr.map.len()).as_bytes());
                for (key, &element) in arr.map.iter() {
                    self.key(key);
                    self.value(vm, element)?;
                }
                self.out.push(b'}');
            }
            Val::ConstArray(_) => {
                // Constant arrays only exist as compile-time templates.
                let materialized = vm.arena.alloc(val.clone());
                let tmp = materialize(vm, materialized);
                self.n -= 1;
                return self.value(vm, tmp);
            }
            Val::Object(payload) => self.object(vm, handle, payload)?,
            Val::ObjPayload(_) => return Err("serialize(): invalid object".into()),
        }
        Ok(())
    }

    fn object(&mut self, vm: &mut VM, handle: Handle, payload: Handle) -> Result<(), String> {
        let (class, props) = match &vm.arena.get(payload).value {
            Val::ObjPayload(obj) => (obj.class, obj.properties.clone()),
            _ => return Err("serialize(): invalid object".into()),
        };
        let class_name = vm.context.interner.lookup(class).unwrap_or(b"").to_vec();
        let def = vm.context.classes.get(&class);

        if def.is_some_and(|d| d.is_enum) {
            let case = def
                .and_then(|d| d.enum_cases.iter().find(|c| vm.arena.get(c.handle).value == Val::Object(payload)))
                .map(|c| vm.context.interner.lookup(c.name).unwrap_or(b"").to_vec())
                .unwrap_or_default();
            let mut name = class_name.clone();
            name.push(b':');
            name.extend_from_slice(&case);
            self.out.extend_from_slice(format!("E:{}:\"", name.len()).as_bytes());
            self.out.extend_from_slice(&name);
            self.out.extend_from_slice(b"\";");
            return Ok(());
        }
        if class_name == b"Closure" || class_name.windows(10).any(|w| w == b"@anonymous") {
            let shown = class_name.split(|b| *b == 0).next().unwrap_or(b"");
            let msg = format!("Serialization of '{}' is not allowed", String::from_utf8_lossy(shown));
            return Err(vm.throw_from_native(b"Exception", &msg));
        }

        let sym = |vm: &mut VM, name: &[u8]| vm.context.interner.intern(name);

        // __serialize(): array
        let serialize_sym = sym(vm, b"__serialize");
        if vm.find_method(class, serialize_sym).is_some() {
            let data = vm.call_method(handle, serialize_sym, &[]).map_err(|e| vm_error(vm, e))?;
            let Val::Array(arr) = vm.arena.get(data).value.clone() else {
                let msg = format!("{}::__serialize() must return an array", String::from_utf8_lossy(&class_name));
                return Err(vm.throw_from_native(b"TypeError", &msg));
            };
            self.object_header(b'O', &class_name, arr.map.len());
            for (key, &element) in arr.map.iter() {
                self.key(key);
                self.value(vm, element)?;
            }
            self.out.push(b'}');
            return Ok(());
        }

        // Serializable::serialize(): string (deprecated interface)
        let serializable = sym(vm, b"Serializable");
        if vm.is_instance_of(handle, serializable) {
            let method = sym(vm, b"serialize");
            let data = vm.call_method(handle, method, &[]).map_err(|e| vm_error(vm, e))?;
            match vm.arena.get(data).value.clone() {
                Val::String(s) => {
                    self.out.extend_from_slice(
                        format!("C:{}:\"", class_name.len()).as_bytes(),
                    );
                    self.out.extend_from_slice(&class_name);
                    self.out.extend_from_slice(format!("\":{}:{{", s.len()).as_bytes());
                    self.out.extend_from_slice(&s);
                    self.out.push(b'}');
                }
                Val::Null => self.out.extend_from_slice(b"N;"),
                _ => {
                    let msg = format!(
                        "{}::serialize() must return a string or NULL",
                        String::from_utf8_lossy(&class_name)
                    );
                    return Err(vm.throw_from_native(b"Exception", &msg));
                }
            }
            return Ok(());
        }

        // An incomplete class is written back under its original name.
        let incomplete_name_sym = sym(vm, INCOMPLETE_CLASS_NAME);
        let (written_name, mut members): (Vec<u8>, Vec<(Symbol, Handle)>) = if class_name == INCOMPLETE_CLASS {
            let original = props
                .get(&incomplete_name_sym)
                .map(|h| vm.arena.get(*h).value.to_php_string_bytes())
                .unwrap_or_default();
            let rest = props.iter().filter(|(k, _)| **k != incomplete_name_sym).map(|(k, v)| (*k, *v)).collect();
            (original, rest)
        } else {
            (class_name.clone(), props.iter().map(|(k, v)| (*k, *v)).collect())
        };

        // __sleep(): the names of the properties to write
        let sleep_sym = sym(vm, b"__sleep");
        if vm.find_method(class, sleep_sym).is_some() {
            let names = vm.call_method(handle, sleep_sym, &[]).map_err(|e| vm_error(vm, e))?;
            let Val::Array(names) = vm.arena.get(names).value.clone() else {
                vm.report_error(
                    ErrorLevel::Warning,
                    "serialize(): __sleep should return an array only containing the names of instance-variables to serialize",
                );
                self.out.extend_from_slice(b"N;");
                return Ok(());
            };
            let mut chosen = Vec::new();
            for &name_handle in names.map.values() {
                let name = vm.arena.get(name_handle).value.to_php_string_bytes();
                let plain = demangle(&name).1.to_vec();
                let prop = sym(vm, &plain);
                match props.get(&prop) {
                    Some(&h) if !matches!(vm.arena.get(h).value, Val::Uninitialized) => chosen.push((prop, h)),
                    Some(_) => {}
                    None => vm.report_error(
                        ErrorLevel::Warning,
                        &format!(
                            "serialize(): \"{}\" returned as member variable from __sleep() but does not exist",
                            String::from_utf8_lossy(&name)
                        ),
                    ),
                }
            }
            members = chosen;
        }

        members.retain(|(_, h)| !matches!(vm.arena.get(*h).value, Val::Uninitialized));
        self.object_header(b'O', &written_name, members.len());
        for (prop, value) in members {
            let name = mangled_name(vm, class, prop);
            self.string(&name);
            self.value(vm, value)?;
        }
        self.out.push(b'}');
        Ok(())
    }

    fn object_header(&mut self, tag: u8, class_name: &[u8], count: usize) {
        self.out.push(tag);
        self.out
            .extend_from_slice(format!(":{}:\"", class_name.len()).as_bytes());
        self.out.extend_from_slice(class_name);
        self.out.extend_from_slice(format!("\":{}:{{", count).as_bytes());
    }
}

/// The declaring class and visibility of `prop` as seen from `class`.
fn property_owner(vm: &VM, class: Symbol, prop: Symbol) -> Option<(Symbol, Visibility)> {
    vm.walk_inheritance_chain(class, |def: &ClassDef, cls| {
        def.properties.get(&prop).map(|entry| (cls, entry.visibility))
    })
}

/// A property's serialized name: `name`, `\0*\0name` (protected) or `\0Class\0name` (private).
fn mangled_name(vm: &VM, class: Symbol, prop: Symbol) -> Vec<u8> {
    let name = vm.context.interner.lookup(prop).unwrap_or(b"").to_vec();
    match property_owner(vm, class, prop) {
        Some((_, Visibility::Protected)) => [b"\0*\0".as_slice(), &name].concat(),
        Some((owner, Visibility::Private)) => {
            let owner = vm.context.interner.lookup(owner).unwrap_or(b"");
            [b"\0".as_slice(), owner, b"\0", &name].concat()
        }
        _ => name,
    }
}

/// Split a mangled property name into (class or "*", name).
fn demangle(name: &[u8]) -> (Option<&[u8]>, &[u8]) {
    if name.first() == Some(&0)
        && let Some(end) = name[1..].iter().position(|b| *b == 0)
    {
        return (Some(&name[1..1 + end]), &name[2 + end..]);
    }
    (None, name)
}

/// A runtime copy of a compile-time constant array.
fn materialize(vm: &mut VM, handle: Handle) -> Handle {
    let val = vm.arena.get(handle).value.clone();
    match val {
        Val::ConstArray(items) => {
            let mut arr = ArrayData::new();
            for (key, item) in items.iter() {
                let key = match key {
                    crate::core::value::ConstArrayKey::Int(i) => ArrayKey::Int(*i),
                    crate::core::value::ConstArrayKey::Str(s) => ArrayKey::Str(s.clone()),
                };
                let h = vm.arena.alloc(item.clone());
                let h = materialize(vm, h);
                arr.insert(key, h);
            }
            vm.arena.alloc(Val::Array(arr.into()))
        }
        _ => handle,
    }
}

/// See VM::callback_error.
fn vm_error(vm: &mut VM, e: VmError) -> String {
    vm.callback_error(e)
}

// ---------------------------------------------------------------------------
// unserialize
// ---------------------------------------------------------------------------

enum Failure {
    /// Malformed input: PHP warns "Error at offset" and returns false.
    Syntax { offset: usize, eof: bool },
    /// An error to raise as is (an exception from user code, a TypeError, ...).
    Php(String),
}

enum Allowed {
    All,
    None,
    Only(Vec<Vec<u8>>),
}

struct Options {
    allowed: Allowed,
    max_depth: usize,
}

impl Options {
    fn read(vm: &mut VM, options: Option<Handle>) -> Result<Self, String> {
        let mut opts = Options {
            allowed: Allowed::All,
            max_depth: 4096,
        };
        let Some(options) = options else { return Ok(opts) };
        let Val::Array(arr) = vm.arena.get(options).value.clone() else {
            return Ok(opts);
        };
        if let Some(&h) = arr.map.get(&ArrayKey::Str(Rc::new(b"allowed_classes".to_vec()))) {
            opts.allowed = match vm.arena.get(h).value.clone() {
                Val::Bool(true) => Allowed::All,
                Val::Bool(false) => Allowed::None,
                Val::Array(names) => Allowed::Only(
                    names
                        .map
                        .values()
                        .map(|n| vm.arena.get(*n).value.to_php_string_bytes().to_ascii_lowercase())
                        .collect(),
                ),
                _ => {
                    return Err(vm.throw_from_native(
                        b"TypeError",
                        "unserialize(): Option \"allowed_classes\" must be an array of class names, bool given",
                    ));
                }
            };
        }
        if let Some(&h) = arr.map.get(&ArrayKey::Str(Rc::new(b"max_depth".to_vec()))) {
            let depth = vm.arena.get(h).value.to_int();
            if depth < 0 {
                return Err(vm.throw_from_native(
                    b"ValueError",
                    "unserialize(): Option \"max_depth\" must be greater than or equal to 0",
                ));
            }
            if depth > 0 {
                opts.max_depth = depth as usize;
            }
        }
        Ok(opts)
    }

    fn allows(&self, class: &[u8]) -> bool {
        match &self.allowed {
            Allowed::All => true,
            Allowed::None => false,
            Allowed::Only(names) => names.contains(&class.to_ascii_lowercase()),
        }
    }
}

struct Unserializer<'a> {
    data: &'a [u8],
    pos: usize,
    /// Values by number, for `R:` and `r:`.
    vars: Vec<Handle>,
    /// __unserialize/__wakeup calls, run in order once the whole input parsed.
    deferred: Vec<(Handle, Option<Handle>)>,
    options: Options,
    depth: usize,
}

type Parse<T> = Result<T, Failure>;

impl Unserializer<'_> {
    fn fail(&self, offset: usize) -> Failure {
        Failure::Syntax {
            offset,
            eof: self.pos >= self.data.len(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn expect(&mut self, byte: u8, start: usize) -> Parse<()> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.fail(start))
        }
    }

    /// Bytes up to (not including) `delim`, consuming the delimiter.
    fn until(&mut self, delim: u8, start: usize) -> Parse<&[u8]> {
        let rest = &self.data[self.pos..];
        let Some(len) = rest.iter().position(|b| *b == delim) else {
            self.pos = self.data.len();
            return Err(self.fail(start));
        };
        let out = &self.data[self.pos..self.pos + len];
        self.pos += len + 1;
        Ok(out)
    }

    fn int(&mut self, delim: u8, start: usize) -> Parse<i64> {
        let text = self.until(delim, start)?;
        let ok = !text.is_empty()
            && text.iter().enumerate().all(|(i, b)| b.is_ascii_digit() || (i == 0 && (*b == b'-' || *b == b'+')));
        if !ok {
            return Err(self.fail(start));
        }
        // Out-of-range integers saturate the way ZEND_STRTOL does.
        let text = std::str::from_utf8(text).unwrap_or("0");
        Ok(text.parse::<i64>().unwrap_or(if text.starts_with('-') { i64::MIN } else { i64::MAX }))
    }

    fn len(&mut self, start: usize) -> Parse<usize> {
        let n = self.int(b':', start)?;
        usize::try_from(n).map_err(|_| self.fail(start))
    }

    /// `"<len bytes>"`
    fn quoted(&mut self, len: usize, start: usize) -> Parse<Vec<u8>> {
        self.expect(b'"', start)?;
        if self.pos + len > self.data.len() {
            self.pos = self.data.len();
            return Err(self.fail(start));
        }
        let out = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        self.expect(b'"', start)?;
        Ok(out)
    }

    fn key(&mut self, vm: &mut VM) -> Parse<ArrayKey> {
        let start = self.pos;
        let tag = self.peek().ok_or_else(|| self.fail(start))?;
        self.pos += 1;
        self.expect(b':', start)?;
        match tag {
            b'i' => Ok(ArrayKey::Int(self.int(b';', start)?)),
            b's' => {
                let len = self.len(start)?;
                let s = self.quoted(len, start)?;
                self.expect(b';', start)?;
                // Numeric string keys become integer keys, as in any array.
                let key_val = Val::String(Rc::new(s.clone()));
                Ok(vm.array_key_from_value(&key_val).unwrap_or(ArrayKey::Str(Rc::new(s))))
            }
            _ => Err(self.fail(start)),
        }
    }

    fn value(&mut self, vm: &mut VM, _nested: bool) -> Parse<Handle> {
        let start = self.pos;
        let tag = self.peek().ok_or_else(|| self.fail(start))?;
        self.pos += 1;

        if tag == b'N' {
            self.expect(b';', start)?;
            return Ok(self.push(vm.arena.alloc(Val::Null)));
        }
        self.expect(b':', start)?;
        match tag {
            b'b' => {
                let v = self.int(b';', start)?;
                if v != 0 && v != 1 {
                    return Err(self.fail(start));
                }
                Ok(self.push(vm.arena.alloc(Val::Bool(v == 1))))
            }
            b'i' => {
                let v = self.int(b';', start)?;
                Ok(self.push(vm.arena.alloc(Val::Int(v))))
            }
            b'd' => {
                let text = self.until(b';', start)?;
                let f = match text {
                    b"INF" => f64::INFINITY,
                    b"-INF" => f64::NEG_INFINITY,
                    b"NAN" => f64::NAN,
                    _ => std::str::from_utf8(text)
                        .ok()
                        .and_then(|t| t.parse::<f64>().ok())
                        .ok_or_else(|| self.fail(start))?,
                };
                Ok(self.push(vm.arena.alloc(Val::Float(f))))
            }
            b's' => {
                let len = self.len(start)?;
                let s = self.quoted(len, start)?;
                self.expect(b';', start)?;
                Ok(self.push(vm.arena.alloc(Val::String(Rc::new(s)))))
            }
            b'S' => {
                // Escaped string: \xx hex escapes.
                let len = self.len(start)?;
                self.expect(b'"', start)?;
                let mut out = Vec::with_capacity(len);
                while out.len() < len {
                    let b = self.peek().ok_or_else(|| self.fail(start))?;
                    self.pos += 1;
                    if b == b'\\' {
                        let hex = self.data.get(self.pos..self.pos + 2).ok_or_else(|| self.fail(start))?;
                        let v = u8::from_str_radix(std::str::from_utf8(hex).unwrap_or("x"), 16)
                            .map_err(|_| self.fail(start))?;
                        out.push(v);
                        self.pos += 2;
                    } else {
                        out.push(b);
                    }
                }
                self.expect(b'"', start)?;
                self.expect(b';', start)?;
                Ok(self.push(vm.arena.alloc(Val::String(Rc::new(out)))))
            }
            b'R' | b'r' => {
                let n = self.int(b';', start)?;
                let target = usize::try_from(n)
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|i| self.vars.get(i).copied())
                    .ok_or_else(|| self.fail(start))?;
                if tag == b'R' {
                    // The same slot, now a PHP reference; R: takes no number of its own.
                    vm.arena.get_mut(target).is_ref = true;
                    Ok(target)
                } else {
                    let val = vm.arena.get(target).value.clone();
                    Ok(self.push(vm.arena.alloc(val)))
                }
            }
            b'a' => {
                let count = self.len(start)?;
                self.expect(b'{', start)?;
                let handle = self.push(vm.arena.alloc(Val::Null));
                self.enter(start)?;
                let mut arr = ArrayData::new();
                for _ in 0..count {
                    let key = self.key(vm)?;
                    let element = self.value(vm, true)?;
                    arr.insert(key, element);
                }
                self.depth -= 1;
                self.expect(b'}', self.pos)?;
                vm.arena.get_mut(handle).value = Val::Array(arr.into());
                Ok(handle)
            }
            b'O' => self.object(vm, start),
            b'C' => self.custom_object(vm, start),
            b'E' => {
                let len = self.len(start)?;
                let name = self.quoted(len, start)?;
                self.expect(b';', start)?;
                let Some(colon) = name.iter().position(|b| *b == b':') else {
                    return Err(self.fail(start));
                };
                let class = vm.context.interner.intern(&name[..colon]);
                let case = vm.context.interner.intern(&name[colon + 1..]);
                if !vm.context.classes.contains_key(&class) {
                    let _ = vm.trigger_autoload(class);
                }
                let found = vm
                    .context
                    .classes
                    .get(&class)
                    .filter(|d| d.is_enum)
                    .and_then(|d| d.enum_cases.iter().find(|c| c.name == case))
                    .map(|c| c.handle);
                match found {
                    Some(h) => {
                        let val = vm.arena.get(h).value.clone();
                        Ok(self.push(vm.arena.alloc(val)))
                    }
                    None => Err(self.fail(start)),
                }
            }
            _ => Err(self.fail(start)),
        }
    }

    fn push(&mut self, handle: Handle) -> Handle {
        self.vars.push(handle);
        handle
    }

    fn enter(&mut self, start: usize) -> Parse<()> {
        self.depth += 1;
        if self.depth > self.options.max_depth {
            return Err(Failure::Php(format!(
                "unserialize(): Maximum depth of {} exceeded at offset {}",
                self.options.max_depth, start
            )));
        }
        Ok(())
    }

    /// A new object of `class` without running its constructor, or an
    /// __PHP_Incomplete_Class standing in for a class that is unknown or not allowed.
    fn instantiate(&mut self, vm: &mut VM, class_name: &[u8]) -> (Handle, Handle, bool) {
        let mut class = vm.context.interner.intern(class_name);
        if self.options.allows(class_name) && !vm.context.classes.contains_key(&class) {
            let _ = vm.trigger_autoload(class);
        }
        let complete = self.options.allows(class_name) && vm.context.classes.contains_key(&class);
        if !complete {
            class = vm.context.interner.intern(INCOMPLETE_CLASS);
        }
        let properties = if complete {
            vm.collect_properties(class, PropertyCollectionMode::All)
        } else {
            let name_sym = vm.context.interner.intern(INCOMPLETE_CLASS_NAME);
            let name = vm.arena.alloc(Val::String(Rc::new(class_name.to_vec())));
            std::iter::once((name_sym, name)).collect()
        };
        let payload = vm.arena.alloc(Val::ObjPayload(ObjectData {
            class,
            properties,
            internal: None,
            dynamic_properties: Default::default(),
            id: 0,
        }));
        (vm.arena.alloc(Val::Object(payload)), payload, complete)
    }

    fn class_name(&mut self, start: usize) -> Parse<Vec<u8>> {
        let len = self.len(start)?;
        let name = self.quoted(len, start)?;
        self.expect(b':', start)?;
        let valid = !name.is_empty()
            && name
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'\\' || *b >= 0x80);
        if !valid {
            return Err(self.fail(start));
        }
        Ok(name)
    }

    fn object(&mut self, vm: &mut VM, start: usize) -> Parse<Handle> {
        let class_name = self.class_name(start)?;
        let count = self.len(start)?;
        self.expect(b'{', start)?;
        let (handle, payload, complete) = self.instantiate(vm, &class_name);
        self.push(handle);
        self.enter(start)?;

        let class = match &vm.arena.get(payload).value {
            Val::ObjPayload(obj) => obj.class,
            _ => return Err(self.fail(start)),
        };
        let unserialize_sym = vm.context.interner.intern(b"__unserialize");
        let has_unserialize = complete && vm.find_method(class, unserialize_sym).is_some();

        let mut data = ArrayData::new();
        for _ in 0..count {
            let key = self.key(vm)?;
            let value = self.value(vm, true)?;
            if has_unserialize {
                data.insert(key, value);
                continue;
            }
            let name = match &key {
                ArrayKey::Int(i) => i.to_string().into_bytes(),
                ArrayKey::Str(s) => demangle(s).1.to_vec(),
            };
            let prop = vm.context.interner.intern(&name);
            if let Val::ObjPayload(obj) = &mut vm.arena.get_mut(payload).value {
                if !obj.properties.contains_key(&prop) {
                    obj.dynamic_properties.insert(prop);
                }
                obj.properties.insert(prop, value);
            }
        }
        self.depth -= 1;
        self.expect(b'}', self.pos)?;

        if has_unserialize {
            let arr = vm.arena.alloc(Val::Array(data.into()));
            self.deferred.push((handle, Some(arr)));
        } else if complete {
            self.deferred.push((handle, None));
        }
        Ok(handle)
    }

    /// `C:len:"Class":len:{data}` for classes implementing Serializable.
    fn custom_object(&mut self, vm: &mut VM, start: usize) -> Parse<Handle> {
        let class_name = self.class_name(start)?;
        let len = self.len(start)?;
        self.expect(b'{', start)?;
        if self.pos + len > self.data.len() {
            self.pos = self.data.len();
            return Err(self.fail(start));
        }
        let payload_data = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        self.expect(b'}', start)?;
        let (handle, _, complete) = self.instantiate(vm, &class_name);
        self.push(handle);
        if complete {
            let method = vm.context.interner.intern(b"unserialize");
            let arg = vm.arena.alloc(Val::String(Rc::new(payload_data)));
            vm.call_method(handle, method, &[arg])
                .map_err(|e| Failure::Php(vm_error(vm, e)))?;
        }
        Ok(handle)
    }

    fn run_deferred(&mut self, vm: &mut VM) -> Result<(), String> {
        let unserialize_sym = vm.context.interner.intern(b"__unserialize");
        let wakeup_sym = vm.context.interner.intern(b"__wakeup");
        for (obj, data) in std::mem::take(&mut self.deferred) {
            let class = match &vm.arena.get(obj).value {
                Val::Object(p) => match &vm.arena.get(*p).value {
                    Val::ObjPayload(o) => o.class,
                    _ => continue,
                },
                _ => continue,
            };
            match data {
                Some(arr) => {
                    vm.call_method(obj, unserialize_sym, &[arr]).map_err(|e| vm_error(vm, e))?;
                }
                None if vm.find_method(class, wakeup_sym).is_some() => {
                    vm.call_method(obj, wakeup_sym, &[]).map_err(|e| vm_error(vm, e))?;
                }
                None => {}
            }
        }
        Ok(())
    }
}
