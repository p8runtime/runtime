//! serialize and unserialize: PHP's format, objects' back references
//! (`r:N;`), __serialize/__unserialize, __sleep/__wakeup, enums,
//! __PHP_Incomplete_Class for classes not there or not allowed.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/var.c (php_var_serialize),
//! $PHP_SRC_PATH/ext/standard/var_unserializer.re

use crate::arg;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::number::{Precision, float_to_string};
use p8_runtime::object::{CLASS_ENUM, Object, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "serialize", run: serialize, by_ref: &[] },
    Builtin { name: "unserialize", run: unserialize, by_ref: &[] },
];

fn text(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

fn method(o: &Rc<Object>, name: &[u8]) -> Value {
    Value::Array(Rc::new(PhpArray::from_list(vec![Value::Object(Rc::clone(o)), Value::Str(PhpStr::from(name))])))
}

struct Writer<'c> {
    ctx: &'c mut dyn Ctx,
    out: Vec<u8>,
    /// Values written so far (r:N counts them from 1).
    count: usize,
    objects: Vec<(*const Object, usize)>,
}

impl Writer<'_> {
    fn string(&mut self, s: &[u8]) {
        self.out.extend_from_slice(format!("s:{}:\"", s.len()).as_bytes());
        self.out.extend_from_slice(s);
        self.out.extend_from_slice(b"\";");
    }

    fn key(&mut self, k: &Key) {
        match k {
            Key::Int(i) => self.out.extend_from_slice(format!("i:{i};").as_bytes()),
            Key::Str(s) => self.string(s.as_bytes()),
        }
    }

    fn value(&mut self, v: &Value) -> Result<(), Throw> {
        self.count += 1;
        match v {
            Value::Null | Value::Undef => self.out.extend_from_slice(b"N;"),
            Value::Bool(b) => self.out.extend_from_slice(if *b { b"b:1;" } else { b"b:0;" }),
            Value::Int(i) => self.out.extend_from_slice(format!("i:{i};").as_bytes()),
            Value::Float(f) => {
                self.out.extend_from_slice(b"d:");
                self.out.extend_from_slice(float_to_string(*f, Precision::SERIALIZE, 'E').as_bytes());
                self.out.push(b';');
            }
            Value::Str(s) => self.string(s.as_bytes()),
            Value::Ref(cell) => {
                self.count -= 1;
                let inner = cell.borrow().clone();
                self.value(&inner)?;
            }
            Value::Array(a) => {
                self.out.extend_from_slice(format!("a:{}:{{", a.len()).as_bytes());
                for (k, v) in a.iter() {
                    self.key(&k);
                    self.value(v)?;
                }
                self.out.push(b'}');
            }
            Value::Object(o) => self.object(o)?,
        }
        Ok(())
    }

    fn object(&mut self, o: &Rc<Object>) -> Result<(), Throw> {
        // A lazy object is initialized first (unless it asked not to be).
        let lazy;
        let o = if o.lazy.get() != 0 && o.lazy_info.borrow().as_ref().is_none_or(|l| l.options & 8 == 0 || !o.is_uninit_lazy()) {
            lazy = self.ctx.lazy_initialize(o)?;
            &lazy
        } else {
            o
        };
        let ptr = Rc::as_ptr(o);
        if let Some(&(_, n)) = self.objects.iter().find(|(p, _)| *p == ptr) {
            self.out.extend_from_slice(format!("r:{n};").as_bytes());
            return Ok(());
        }
        self.objects.push((ptr, self.count));
        let name = o.class.name.as_bytes().to_vec();
        if o.class.flags & CLASS_ENUM != 0 {
            let case = o.class.read(o, b"name", &o.class.lower).to_php_string().unwrap_or_else(PhpStr::empty);
            let body = [name.as_slice(), b":", case.as_bytes()].concat();
            self.out.extend_from_slice(format!("E:{}:\"", body.len()).as_bytes());
            self.out.extend_from_slice(&body);
            self.out.extend_from_slice(b"\";");
            return Ok(());
        }
        if o.class.lower == b"closure" || o.class.lower == b"generator" || o.class.flags & p8_runtime::object::CLASS_NOT_SERIALIZABLE != 0 {
            return Err(self.ctx.throw("Exception", format!("Serialization of '{}' is not allowed", text(&name))));
        }
        let header = |w: &mut Self, n: usize| {
            w.out.extend_from_slice(format!("O:{}:\"", name.len()).as_bytes());
            w.out.extend_from_slice(&name);
            w.out.extend_from_slice(format!("\":{n}:{{").as_bytes());
        };
        if o.class.method(b"__serialize").is_some() {
            let data = self.ctx.call(&method(o, b"__serialize"), &mut [])?;
            let Value::Array(a) = data else {
                return Err(self.ctx.throw("TypeError", format!("{}::__serialize() must return an array", text(&name))));
            };
            header(self, a.len());
            for (k, v) in a.iter() {
                self.key(&k);
                self.value(v)?;
            }
            self.out.push(b'}');
            return Ok(());
        }
        // __sleep: only the properties it names.
        let only: Option<Vec<PhpStr>> = if o.class.method(b"__sleep").is_some() {
            match self.ctx.call(&method(o, b"__sleep"), &mut [])? {
                Value::Array(a) => Some(a.values().map(|v| v.to_php_string().unwrap_or_else(PhpStr::empty)).collect()),
                _ => {
                    self.ctx.warn("serialize(): __sleep should return an array only containing the names of instance-variables to serialize");
                    self.out.extend_from_slice(b"N;");
                    return Ok(());
                }
            }
        } else {
            None
        };
        // Every property by its serialized (mangled) name.
        let mut all: Vec<(Vec<u8>, Value)> = Vec::new();
        let slots = o.slots.borrow();
        for (p, v) in o.class.properties.iter().zip(slots.iter()) {
            if p.is_internal() || matches!(v, Value::Undef) {
                continue;
            }
            let mangled = match p.visibility {
                Visibility::Public => p.name.as_bytes().to_vec(),
                Visibility::Protected => [b"\0*\0".as_slice(), p.name.as_bytes()].concat(),
                Visibility::Private => [b"\0", p.declaring_name.as_bytes(), b"\0", p.name.as_bytes()].concat(),
            };
            all.push((mangled, v.clone()));
        }
        drop(slots);
        for (k, v) in o.dynamic.borrow().iter() {
            let k = match k {
                Key::Int(i) => i.to_string().into_bytes(),
                Key::Str(s) => s.as_bytes().to_vec(),
            };
            all.push((k, v.clone()));
        }
        let props: Vec<(Vec<u8>, Value)> = match only {
            None => all,
            // __sleep's names in its order, each the first of: the public
            // property, the object's class's private one, the protected one
            // (a name given mangled is that property); one there is none of
            // warns and is left out (php_var_serialize_get_sleep_props).
            Some(names) => {
                let find = |key: &[u8]| all.iter().find(|(k, _)| k.as_slice() == key).cloned();
                let mut out = Vec::new();
                for name in names {
                    let n = name.as_bytes();
                    let private = [b"\0", o.class.name.as_bytes(), b"\0", n].concat();
                    let protected = [b"\0*\0".as_slice(), n].concat();
                    match find(n).or_else(|| find(&private)).or_else(|| find(&protected)) {
                        Some(found) => out.push(found),
                        None => self.ctx.warn(&format!("serialize(): \"{}\" returned as member variable from __sleep() but does not exist", text(n))),
                    }
                }
                out
            }
        };
        header(self, props.len());
        for (k, v) in props {
            self.string(&k);
            self.value(&v)?;
        }
        self.out.push(b'}');
        Ok(())
    }
}

/// `v` as serialize() writes it.
pub(crate) fn serialized(ctx: &mut dyn Ctx, v: &Value) -> Result<Vec<u8>, Throw> {
    let mut w = Writer { ctx, out: Vec::new(), count: 0, objects: Vec::new() };
    w.value(v)?;
    Ok(w.out)
}

/// A value read from the start of `data` as unserialize() reads it, and the
/// offset after it; None where it is not one.
pub(crate) fn read_value(ctx: &mut dyn Ctx, data: &[u8]) -> Option<(Value, usize)> {
    let mut r = Reader { s: data, at: 0, values: Vec::new(), allowed: None, wakeups: Vec::new(), missing: Vec::new() };
    let v = r.value(ctx).ok()?;
    Some((v, r.at))
}

fn serialize(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    let mut w = Writer { ctx, out: Vec::new(), count: 0, objects: Vec::new() };
    w.value(&v)?;
    Ok(Value::Str(PhpStr::new(w.out)))
}

struct Reader<'a> {
    s: &'a [u8],
    at: usize,
    /// Every value read, in order (for r:N and R:N).
    values: Vec<Value>,
    allowed: Option<Vec<Vec<u8>>>,
    wakeups: Vec<(Rc<Object>, Option<PhpArray>)>,
    /// Classes named that are not declared (autoloaded after the read).
    missing: Vec<Vec<u8>>,
}

/// Where unserialize stopped: the offset PHP reports.
struct Bad(usize);

impl Reader<'_> {
    fn expect(&mut self, b: u8) -> Result<(), Bad> {
        if self.s.get(self.at) == Some(&b) {
            self.at += 1;
            Ok(())
        } else {
            Err(Bad(self.at))
        }
    }

    /// Digits up to `end` (a sign allowed).
    fn int(&mut self, end: u8) -> Result<i64, Bad> {
        let start = self.at;
        while self.at < self.s.len() && self.s[self.at] != end {
            self.at += 1;
        }
        let t = std::str::from_utf8(&self.s[start..self.at]).map_err(|_| Bad(start))?;
        let n = t.trim_start_matches('+').parse::<i64>().map_err(|_| Bad(start))?;
        self.expect(end)?;
        Ok(n)
    }

    fn bytes(&mut self) -> Result<Vec<u8>, Bad> {
        let len = self.int(b':')? as usize;
        self.expect(b'"')?;
        let end = self.at + len;
        if end > self.s.len() {
            return Err(Bad(self.at));
        }
        let out = self.s[self.at..end].to_vec();
        self.at = end;
        self.expect(b'"')?;
        Ok(out)
    }

    fn value(&mut self, ctx: &mut dyn Ctx) -> Result<Value, Bad> {
        let start = self.at;
        let Some(&tag) = self.s.get(self.at) else { return Err(Bad(self.at)) };
        if self.s.get(self.at + 1) != Some(&if tag == b'N' { b';' } else { b':' }) {
            return Err(Bad(start));
        }
        self.at += 2;
        let slot = self.values.len();
        self.values.push(Value::Null);
        let v = match tag {
            b'N' => Value::Null,
            b'b' => match self.int(b';')? {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                _ => return Err(Bad(start)),
            },
            b'i' => Value::Int(self.int(b';')?),
            b'd' => {
                let from = self.at;
                while self.at < self.s.len() && self.s[self.at] != b';' {
                    self.at += 1;
                }
                let t = std::str::from_utf8(&self.s[from..self.at]).map_err(|_| Bad(start))?;
                let f = match t {
                    "INF" => f64::INFINITY,
                    "-INF" => f64::NEG_INFINITY,
                    "NAN" => f64::NAN,
                    t => t.parse::<f64>().map_err(|_| Bad(start))?,
                };
                self.expect(b';')?;
                Value::Float(f)
            }
            b's' => {
                let b = self.bytes()?;
                self.expect(b';')?;
                Value::Str(PhpStr::new(b))
            }
            b'r' | b'R' => {
                self.values.pop();
                let n = self.int(b';')? as usize;
                return self.values.get(n.wrapping_sub(1)).cloned().ok_or(Bad(start));
            }
            b'a' => {
                let n = self.int(b':')?;
                self.expect(b'{')?;
                let mut a = PhpArray::new();
                for _ in 0..n {
                    let k = self.key(ctx)?;
                    let v = self.value(ctx)?;
                    a.insert(k, v);
                }
                self.expect(b'}')?;
                Value::Array(Rc::new(a))
            }
            b'E' => {
                let body = self.bytes()?;
                self.expect(b';')?;
                let Some(colon) = body.iter().position(|&c| c == b':') else { return Err(Bad(start)) };
                let class = ctx.class(&body[..colon]).filter(|c| c.flags & CLASS_ENUM != 0).ok_or(Bad(start))?;
                if !class.enum_cases.iter().any(|c| c.as_bytes() == &body[colon + 1..]) {
                    return Err(Bad(start));
                }
                ctx.enum_case(&class, &body[colon + 1..])
            }
            b'O' => {
                let name = self.bytes()?;
                self.expect(b':')?;
                let n = self.int(b':')?;
                self.expect(b'{')?;
                let allowed = self.allowed.as_ref().is_none_or(|list| list.iter().any(|c| c.eq_ignore_ascii_case(&name)));
                let class = if allowed { ctx.class(&name) } else { None };
                if allowed && class.is_none() {
                    self.missing.push(name.clone());
                }
                let (o, incomplete) = match class {
                    Some(c) => (ctx.new_object(&c), false),
                    None => {
                        let c = ctx.class(b"__PHP_Incomplete_Class").ok_or(Bad(start))?;
                        let o = ctx.new_object(&c);
                        o.dynamic.borrow_mut().insert(Key::from_string(PhpStr::from("__PHP_Incomplete_Class_Name")), Value::Str(PhpStr::new(name)));
                        (o, true)
                    }
                };
                self.values[slot] = Value::Object(Rc::clone(&o));
                let custom = !incomplete && o.class.method(b"__unserialize").is_some();
                let mut data = PhpArray::new();
                for _ in 0..n {
                    let k = self.key(ctx)?;
                    let v = self.value(ctx)?;
                    if custom {
                        data.insert(k, v);
                    } else {
                        set_property(&o, k, v);
                    }
                }
                self.expect(b'}')?;
                if custom {
                    self.wakeups.push((Rc::clone(&o), Some(data)));
                } else if !incomplete && o.class.method(b"__wakeup").is_some() {
                    self.wakeups.push((Rc::clone(&o), None));
                }
                Value::Object(o)
            }
            _ => return Err(Bad(start)),
        };
        self.values[slot] = v.clone();
        Ok(v)
    }

    fn key(&mut self, ctx: &mut dyn Ctx) -> Result<Key, Bad> {
        let at = self.at;
        let k = self.value(ctx)?;
        self.values.pop();
        match k {
            Value::Int(i) => Ok(Key::Int(i)),
            Value::Str(s) => Ok(Key::from_string(s)),
            _ => Err(Bad(at)),
        }
    }
}

/// A property from its serialized name: "\0*\0p" protected, "\0C\0p"
/// private to C, else public (or dynamic).
fn set_property(o: &Object, key: Key, v: Value) {
    let raw = match &key {
        Key::Int(i) => i.to_string().into_bytes(),
        Key::Str(s) => s.as_bytes().to_vec(),
    };
    let (declaring, name): (Option<&[u8]>, &[u8]) = if raw.first() == Some(&0) {
        match raw[1..].iter().position(|&c| c == 0) {
            Some(end) => (Some(&raw[1..1 + end]), &raw[2 + end..]),
            None => (None, &raw[..]),
        }
    } else {
        (None, &raw[..])
    };
    let props = &o.class.properties;
    let slot = match declaring {
        Some(b"*") => props.iter().position(|p| p.name.as_bytes() == name && p.visibility == Visibility::Protected),
        Some(c) => props.iter().position(|p| p.name.as_bytes() == name && p.visibility == Visibility::Private && p.declaring_name.as_bytes().eq_ignore_ascii_case(c)),
        // An unmangled name is the declared property of that name, whatever
        // its visibility (php_var_unserialize: the class's property info),
        // the most derived one (properties run parent first).
        None => {
            let named = |vis: Visibility| props.iter().rposition(|p| p.name.as_bytes() == name && p.visibility == vis);
            named(Visibility::Public).or_else(|| named(Visibility::Protected)).or_else(|| named(Visibility::Private))
        }
    };
    match slot {
        Some(i) => o.slots.borrow_mut()[i] = v,
        None => {
            o.dynamic.borrow_mut().insert(Key::from_string(PhpStr::from(name)), v);
        }
    }
}

/// unserialize(string $data, array $options = [])
fn unserialize(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let data = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    let allowed = match arg(args, 1) {
        Value::Array(o) => match o.get(&Key::from_string(PhpStr::from("allowed_classes"))).map(Value::deref) {
            Some(Value::Bool(false)) => Some(Vec::new()),
            Some(Value::Array(list)) => Some(list.values().map(|v| v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()).collect()),
            _ => None,
        },
        _ => None,
    };
    if data.is_empty() {
        return Ok(Value::Bool(false));
    }
    let mut r = Reader { s: data.as_bytes(), at: 0, values: Vec::new(), allowed, wakeups: Vec::new(), missing: Vec::new() };
    match r.value(ctx) {
        Ok(v) => {
            // A class it names that is not declared yet is autoloaded, and
            // the call runs again with it (one still missing is incomplete).
            for name in std::mem::take(&mut r.missing) {
                ctx.class_load(&name)?;
            }
            for (o, data) in std::mem::take(&mut r.wakeups) {
                match data {
                    Some(d) => {
                        ctx.call(&method(&o, b"__unserialize"), &mut [Value::Array(Rc::new(d))])?;
                    }
                    None => {
                        ctx.call(&method(&o, b"__wakeup"), &mut [])?;
                    }
                }
            }
            Ok(v)
        }
        Err(Bad(at)) => {
            ctx.warn(&format!("unserialize(): Error at offset {at} of {} bytes", data.len()));
            Ok(Value::Bool(false))
        }
    }
}
