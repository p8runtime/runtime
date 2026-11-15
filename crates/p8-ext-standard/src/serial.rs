//! serialize and unserialize: PHP's format, objects' back references
//! (`r:N;`), __serialize/__unserialize, __sleep/__wakeup, enums,
//! __PHP_Incomplete_Class for classes not there or not allowed.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/var.c (php_var_serialize),
//! $PHP_SRC_PATH/ext/standard/var_unserializer.re

use crate::arg;
use p8_runtime::array::{EntryHere, Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::number::{Precision, float_to_string};
use p8_runtime::object::{CLASS_ENUM, CLASS_RESOURCE, Class, Object, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::{Kind, Value};
use std::cell::RefCell;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "serialize", run: serialize, by_ref: &[] },
    Builtin { name: "unserialize", run: unserialize, by_ref: &[] },
];

fn text(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

fn method(o: &Rc<Object>, name: &[u8]) -> Value {
    Value::array(PhpArray::from_list(vec![Value::object(Rc::clone(o)), Value::str(PhpStr::from(name))]))
}

struct Writer<'c> {
    /// None: writing from the arguments alone (serialize()'s quick
    /// result), which stops at the first object ([`Halt::Object`]).
    ctx: Option<&'c mut dyn Ctx>,
    out: Vec<u8>,
    /// Values written so far (r:N and R:N count them from 1).
    count: usize,
    objects: Vec<(*const Object, usize)>,
    /// The references written, each with its number (held: a cell freed
    /// while writing cannot have its address taken for one written).
    refs: Vec<(Rc<RefCell<Value>>, usize)>,
}

/// Why writing stopped.
enum Halt {
    /// What PHP code (__serialize, __sleep, a lazy object's initializer)
    /// threw, or an error.
    Threw(Throw),
    /// An object, met with no Ctx to write it with.
    Object,
}

impl From<Throw> for Halt {
    fn from(t: Throw) -> Self {
        Halt::Threw(t)
    }
}

impl Halt {
    /// The Throw of a Writer that has a Ctx (it never halts at an object).
    fn thrown(self) -> Throw {
        match self {
            Halt::Threw(t) => t,
            Halt::Object => unreachable!("a Writer with a Ctx writes every object"),
        }
    }
}

impl<'c> Writer<'c> {
    fn new(ctx: Option<&'c mut dyn Ctx>, out: Vec<u8>) -> Self {
        Writer { ctx, out, count: 0, objects: Vec::new(), refs: Vec::new() }
    }

    /// The Ctx objects are written with (object() runs only with one:
    /// value() halts at an object without one).
    fn ctx(&mut self) -> &mut dyn Ctx {
        match &mut self.ctx {
            Some(ctx) => &mut **ctx,
            None => unreachable!("value() halts at an object when there is no Ctx"),
        }
    }

    /// `tag`, `n`'s decimal digits, then `end` (`i:7;`, `a:3:{`): built
    /// in a buffer of its own, which is appended whole (a copy of a fixed
    /// size, no call) and cut back to the bytes written.
    fn tagged<const E: usize>(&mut self, tag: &[u8; 2], n: i64, end: &[u8; E]) {
        // (2 + a sign + 19 digits + an `end` of at most 2: at most 24.)
        let mut buf = [0u8; 24];
        buf[..2].copy_from_slice(tag);
        let mut m = n.unsigned_abs();
        let sign = usize::from(n < 0);
        buf[2] = b'-';
        let stop = if m < 10 {
            // (Most lengths and many ints: one digit, no count of them.)
            buf[2 + sign] = b'0' + m as u8;
            3 + sign
        } else {
            let stop = 3 + sign + m.ilog10() as usize;
            let mut at = stop;
            while m != 0 {
                at -= 1;
                buf[at] = b'0' + (m % 10) as u8;
                m /= 10;
            }
            stop
        };
        buf[stop..stop + E].copy_from_slice(end);
        let len = self.out.len() + stop + E;
        self.out.extend_from_slice(&buf);
        self.out.truncate(len);
    }

    /// A length as written (a Vec's length always fits an i64).
    fn len(n: usize) -> i64 {
        i64::try_from(n).expect("a length fits an i64")
    }

    fn string(&mut self, s: &[u8]) {
        self.tagged(b"s:", Self::len(s.len()), b":\"");
        self.out.extend_from_slice(s);
        self.out.extend_from_slice(b"\";");
    }

    /// `a`'s entries, key then value, in order, and the closing `}` (each
    /// key read in place: a string key's handle is not copied).
    fn entries(&mut self, a: &PhpArray) -> Result<(), Halt> {
        let mut pos = 0;
        loop {
            match a.entry_here(pos) {
                EntryHere::Int(i, v) => {
                    self.tagged(b"i:", i, b";");
                    self.value(v)?;
                }
                EntryHere::Str(s, v) => {
                    self.string(s.as_bytes());
                    self.value(v)?;
                }
                EntryHere::Hole => {}
                EntryHere::End => break,
            }
            pos += 1;
        }
        self.out.push(b'}');
        Ok(())
    }

    fn value(&mut self, v: &Value) -> Result<(), Halt> {
        self.count += 1;
        match v.kind() {
            Kind::Null | Kind::Undef => self.out.extend_from_slice(b"N;"),
            Kind::Bool => self.out.extend_from_slice(if v.as_bool().expect("a bool") { b"b:1;" } else { b"b:0;" }),
            Kind::Int => {
                self.tagged(b"i:", v.as_int().expect("an int"), b";");
            }
            Kind::Float => {
                self.out.extend_from_slice(b"d:");
                self.out.extend_from_slice(float_to_string(v.as_float().expect("a float"), Precision::SERIALIZE, 'E').as_bytes());
                self.out.push(b';');
            }
            Kind::Str => self.string(v.as_str().expect("a string").as_bytes()),
            Kind::Ref => {
                // A reference takes one number (the value in it is not
                // counted again) and is `R:N;` where it is met again; one
                // to an object goes by the object's number; one held in one
                // place only is its value (php_add_var_hash,
                // php_var_serialize_nested_data).
                self.count -= 1;
                let cell = v.as_reference().expect("a reference");
                let inner = cell.borrow().clone();
                if Rc::strong_count(cell) > 1 {
                    let object = inner.as_object().filter(|o| o.class.flags & CLASS_RESOURCE == 0);
                    let seen = match object {
                        Some(o) => self.objects.iter().find(|(p, _)| *p == Rc::as_ptr(o)).map(|&(_, n)| n),
                        None => self.refs.iter().find(|(r, _)| Rc::ptr_eq(r, cell)).map(|&(_, n)| n),
                    };
                    if let Some(n) = seen {
                        self.tagged(b"R:", Self::len(n), b";");
                        return Ok(());
                    }
                    if object.is_none() {
                        self.refs.push((Rc::clone(cell), self.count + 1));
                    }
                }
                self.value(&inner)?;
            }
            Kind::Array => {
                let a = v.as_array().expect("an array");
                self.tagged(b"a:", Self::len(a.len()), b":{");
                self.entries(a)?;
            }
            Kind::Object => {
                let o = v.as_object().expect("an object");
                if o.class.flags & CLASS_RESOURCE != 0 {
                    // a resource is written as 0
                    self.out.extend_from_slice(b"i:0;");
                } else if self.ctx.is_none() {
                    return Err(Halt::Object);
                } else {
                    self.object(o)?;
                }
            }
        }
        Ok(())
    }

    fn object(&mut self, o: &Rc<Object>) -> Result<(), Halt> {
        // A lazy object is initialized first (unless it asked not to be).
        let lazy;
        let o = if o.lazy.get() != 0 && o.lazy_info().as_ref().is_none_or(|l| l.options & 8 == 0 || !o.is_uninit_lazy()) {
            lazy = self.ctx().lazy_initialize(o)?;
            &lazy
        } else {
            o
        };
        let ptr = Rc::as_ptr(o);
        if let Some(&(_, n)) = self.objects.iter().find(|(p, _)| *p == ptr) {
            self.tagged(b"r:", Self::len(n), b";");
            return Ok(());
        }
        self.objects.push((ptr, self.count));
        let name = o.class.name.as_bytes().to_vec();
        if o.class.flags & CLASS_ENUM != 0 {
            let case = o.class.read(o, b"name", &o.class.lower).to_php_string().unwrap_or_else(PhpStr::empty);
            let body = [name.as_slice(), b":", case.as_bytes()].concat();
            self.tagged(b"E:", Self::len(body.len()), b":\"");
            self.out.extend_from_slice(&body);
            self.out.extend_from_slice(b"\";");
            return Ok(());
        }
        if o.class.lower == b"closure" || o.class.lower == b"generator" || o.class.flags & p8_runtime::object::CLASS_NOT_SERIALIZABLE != 0 {
            return Err(self.ctx().throw("Exception", format!("Serialization of '{}' is not allowed", text(&name))).into());
        }
        let header = |w: &mut Self, n: usize| {
            w.tagged(b"O:", Self::len(name.len()), b":\"");
            w.out.extend_from_slice(&name);
            w.tagged(b"\":", Self::len(n), b":{");
        };
        if o.class.method(b"__serialize").is_some() {
            let data = self.ctx().call(&method(o, b"__serialize"), &mut [])?;
            let Ok(a) = data.into_array() else {
                return Err(self.ctx().throw("TypeError", format!("{}::__serialize() must return an array", text(&name))).into());
            };
            header(self, a.len());
            return self.entries(&a);
        }
        // __sleep: only the properties it names.
        let only: Option<Vec<PhpStr>> = if o.class.method(b"__sleep").is_some() {
            match self.ctx().call(&method(o, b"__sleep"), &mut [])?.into_array() {
                Ok(a) => Some(a.values().map(|v| v.to_php_string().unwrap_or_else(PhpStr::empty)).collect()),
                Err(_) => {
                    self.ctx().warn("serialize(): __sleep should return an array only containing the names of instance-variables to serialize");
                    self.out.extend_from_slice(b"N;");
                    return Ok(());
                }
            }
        } else {
            None
        };
        // Every property by its serialized (mangled) name; an uninitialized
        // typed one too (Undef: __sleep may name it, then it is left out).
        let mut all: Vec<(Vec<u8>, Value)> = Vec::new();
        let slots = o.slot_values();
        for (p, v) in o.class.properties.iter().zip(slots.iter()) {
            if p.is_internal() {
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
        for (k, v) in o.dynamic().iter() {
            let k = match k {
                Key::Int(i) => i.to_string().into_bytes(),
                Key::Str(s) => s.as_bytes().to_vec(),
            };
            all.push((k, v.clone()));
        }
        let props: Vec<(Vec<u8>, Value)> = match only {
            None => all.into_iter().filter(|(_, v)| !v.is_undef()).collect(),
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
                        Some((_, v)) if v.is_undef() => {}
                        Some(found) => out.push(found),
                        None => self.ctx().warn(&format!("serialize(): \"{}\" returned as member variable from __sleep() but does not exist", text(n))),
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
    let mut w = Writer::new(Some(ctx), Vec::new());
    w.value(v).map_err(Halt::thrown)?;
    Ok(w.out)
}

/// A value read from the start of `data` as unserialize() reads it, and the
/// offset after it; None where it is not one (Err: what an autoloader
/// threw).
pub(crate) fn read_value(ctx: &mut dyn Ctx, data: &[u8]) -> Result<Option<(Value, usize)>, Throw> {
    let mut r = Reader::new(data, None);
    match r.read(ctx) {
        Ok(v) => Ok(Some((v, r.at))),
        Err(Bad::At(_)) => Ok(None),
        Err(Bad::Threw(t)) => Err(t),
    }
}

/// Values written one after another as serialize() writes one: they share
/// the numbering the `r:N;` and `R:N;` back-references count by (the
/// session extension writes its variables this way).
pub struct Serializer<'c> {
    w: Writer<'c>,
}

impl<'c> Serializer<'c> {
    pub fn new(ctx: &'c mut dyn Ctx) -> Serializer<'c> {
        Serializer { w: Writer::new(Some(ctx), Vec::new()) }
    }

    /// Bytes written as they are (a variable's name and its delimiter).
    pub fn raw(&mut self, bytes: &[u8]) {
        self.w.out.extend_from_slice(bytes);
    }

    pub fn value(&mut self, v: &Value) -> Result<(), Throw> {
        self.w.value(v).map_err(Halt::thrown)
    }

    pub fn finish(self) -> Vec<u8> {
        self.w.out
    }
}

/// Values read one after another from one string as unserialize() reads
/// one, sharing the back-references' numbering (see [`Serializer`]).
pub struct Unserializer<'a> {
    r: Reader<'a>,
}

impl<'a> Unserializer<'a> {
    pub fn new(data: &'a [u8]) -> Unserializer<'a> {
        Unserializer { r: Reader::new(data, None) }
    }

    /// The offset the next value is read from.
    pub fn at(&self) -> usize {
        self.r.at
    }

    /// Continue from `at` (after bytes the caller read itself).
    pub fn seek(&mut self, at: usize) {
        self.r.at = at;
    }

    /// The next value (Ok(Err): the offset it stopped at where there is
    /// none; Err: what an autoloader threw).
    pub fn value(&mut self, ctx: &mut dyn Ctx) -> Result<Result<Value, usize>, Throw> {
        match self.r.read(ctx) {
            Ok(v) => Ok(Ok(v)),
            Err(Bad::At(at)) => Ok(Err(at)),
            Err(Bad::Threw(t)) => Err(t),
        }
    }

    /// The end of the read: the objects it made get their __unserialize()
    /// or __wakeup().
    pub fn finish(&mut self, ctx: &mut dyn Ctx) -> Result<(), Throw> {
        self.r.wake(ctx)
    }
}

fn serialize(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    let mut w = Writer::new(Some(ctx), PhpStr::scratch(64));
    w.value(&v).map_err(Halt::thrown)?;
    Ok(Value::str(PhpStr::from_scratch(w.out)))
}

/// serialize()'s result read from its argument alone, when it holds no
/// object (the only values whose writing can run PHP code, warn or
/// throw): the call is then made with no lent registers and no frame.
/// None for an object anywhere in it (the call is made).
pub(crate) fn serialize_quick(args: &[Value]) -> Option<Value> {
    let [v] = args else { return None };
    // (The argument is a value, as serialize() takes it: a reference
    // passed is the value in it.)
    let inner;
    let v = if v.is_reference() {
        inner = v.deref();
        &inner
    } else {
        v
    };
    let mut w = Writer::new(None, PhpStr::scratch(64));
    w.value(v).ok()?;
    Some(Value::str(PhpStr::from_scratch(w.out)))
}

struct Reader<'a> {
    s: &'a [u8],
    at: usize,
    /// Every value read, in order (for r:N and R:N; `R:N;` is not one).
    values: Vec<Value>,
    /// The numbers (from 0) of the values an `R:N;` in the value being
    /// read names, in order: each is read as a reference, which the `R:`
    /// shares (PHP turns the value into one in place).
    refs: Vec<usize>,
    allowed: Option<Vec<Vec<u8>>>,
    wakeups: Vec<(Rc<Object>, Option<PhpArray>)>,
}

/// Why unserialize stopped: at an offset PHP reports, or an exception
/// (an autoloader's, the unserialize_callback_func's).
enum Bad {
    At(usize),
    Threw(Throw),
}

impl From<Throw> for Bad {
    fn from(t: Throw) -> Bad {
        Bad::Threw(t)
    }
}

impl<'a> Reader<'a> {
    fn new(s: &'a [u8], allowed: Option<Vec<Vec<u8>>>) -> Reader<'a> {
        Reader { s, at: 0, values: Vec::new(), refs: Vec::new(), allowed, wakeups: Vec::new() }
    }

    /// The value at the offset, as unserialize() returns it: where an
    /// `R:N;` names a value in it, that value is read as a reference (the
    /// one the value itself is, if it is that, is the value in it).
    fn read(&mut self, ctx: &mut dyn Ctx) -> Result<Value, Bad> {
        self.refs.clear();
        if self.s[self.at.min(self.s.len())..].windows(2).any(|w| w == b"R:") {
            let mut skim = Reader::new(self.s, None);
            skim.at = self.at;
            let mut count = self.values.len();
            // (Where it is not well formed, the read below says where; the
            // numbers found before that are the ones it can meet.)
            let _ = skim.skim(&mut count, &mut self.refs);
            self.refs.sort_unstable();
            self.refs.dedup();
        }
        let v = self.value(ctx)?;
        Ok(if v.is_reference() { v.deref() } else { v })
    }

    /// The value at the offset read as value() reads it, for the numbers
    /// `R:N;` names alone: nothing is made, looked up or warned about.
    /// `count`: the values before it, numbered as value() numbers them.
    fn skim(&mut self, count: &mut usize, refs: &mut Vec<usize>) -> Result<(), Bad> {
        let Some(&tag) = self.s.get(self.at) else { return Err(Bad::At(self.at)) };
        self.at += 1;
        self.expect(if tag == b'N' { b';' } else { b':' })?;
        if tag == b'R' {
            if let Some(n) = usize::try_from(self.int(b';')?).ok().and_then(|n| n.checked_sub(1)) {
                refs.push(n);
            }
            return Ok(());
        }
        *count += 1;
        match tag {
            b'N' => {}
            b'b' | b'i' | b'r' => {
                self.int(b';')?;
            }
            b'd' => {
                while self.at < self.s.len() && self.s[self.at] != b';' {
                    self.at += 1;
                }
                self.expect(b';')?;
            }
            b's' | b'E' => {
                self.bytes()?;
                self.expect(b';')?;
            }
            b'a' | b'O' => {
                if tag == b'O' {
                    self.bytes()?;
                    self.expect(b':')?;
                }
                let n = self.int(b':')?;
                self.expect(b'{')?;
                for _ in 0..n {
                    // A key (an int or a string) is not counted.
                    if !matches!(self.s.get(self.at), Some(b'i' | b's')) {
                        return Err(Bad::At(self.at));
                    }
                    self.skim(count, refs)?;
                    *count -= 1;
                    self.skim(count, refs)?;
                }
                self.expect(b'}')?;
            }
            _ => return Err(Bad::At(self.at)),
        }
        Ok(())
    }

    /// The number after `r:` or `R:` (digits only, then `;`), as an index
    /// into the values read; Err at `start` where it is not one.
    fn back_number(&mut self, start: usize) -> Result<usize, Bad> {
        if !self.s.get(self.at).is_some_and(u8::is_ascii_digit) {
            return Err(Bad::At(start));
        }
        let n = self.int(b';').map_err(|_| Bad::At(start))?;
        Ok(usize::try_from(n).ok().and_then(|n| n.checked_sub(1)).unwrap_or(usize::MAX))
    }

    /// The objects read get their __unserialize() or __wakeup(), in order,
    /// once the whole value is read (PHP's delayed calls).
    fn wake(&mut self, ctx: &mut dyn Ctx) -> Result<(), Throw> {
        for (o, data) in std::mem::take(&mut self.wakeups) {
            match data {
                Some(d) => {
                    ctx.call(&method(&o, b"__unserialize"), &mut [Value::array(d)])?;
                }
                None => {
                    ctx.call(&method(&o, b"__wakeup"), &mut [])?;
                }
            }
        }
        Ok(())
    }

    fn expect(&mut self, b: u8) -> Result<(), Bad> {
        if self.s.get(self.at) == Some(&b) {
            self.at += 1;
            Ok(())
        } else {
            Err(Bad::At(self.at))
        }
    }

    /// Digits up to `end` (a sign allowed).
    fn int(&mut self, end: u8) -> Result<i64, Bad> {
        let start = self.at;
        while self.at < self.s.len() && self.s[self.at] != end {
            self.at += 1;
        }
        let t = std::str::from_utf8(&self.s[start..self.at]).map_err(|_| Bad::At(start))?;
        let n = t.trim_start_matches('+').parse::<i64>().map_err(|_| Bad::At(start))?;
        self.expect(end)?;
        Ok(n)
    }

    /// A length-prefixed quoted string's bytes, in the input (no copy).
    fn bytes(&mut self) -> Result<&'a [u8], Bad> {
        let len = self.int(b':')? as usize;
        self.expect(b'"')?;
        let end = self.at + len;
        if end > self.s.len() {
            return Err(Bad::At(self.at));
        }
        let s: &'a [u8] = self.s;
        let out = &s[self.at..end];
        self.at = end;
        self.expect(b'"')?;
        Ok(out)
    }

    fn value(&mut self, ctx: &mut dyn Ctx) -> Result<Value, Bad> {
        let start = self.at;
        let Some(&tag) = self.s.get(self.at) else { return Err(Bad::At(self.at)) };
        if self.s.get(self.at + 1) != Some(&if tag == b'N' { b';' } else { b':' }) {
            return Err(Bad::At(start));
        }
        self.at += 2;
        if tag == b'R' {
            // The value it names (a reference read() made it), shared; an
            // `R:N;` is not a value of its own. (Err after it where it names
            // none, as PHP reports.)
            let n = self.back_number(start)?;
            return self.values.get(n).cloned().ok_or(Bad::At(self.at));
        }
        let slot = self.values.len();
        self.values.push(Value::NULL);
        // A value an `R:N;` names is read into a reference made first (an
        // `R:N;` inside it may name it).
        let cell = (!self.refs.is_empty() && self.refs.binary_search(&slot).is_ok()).then(|| {
            let cell = Rc::new(RefCell::new(Value::NULL));
            self.values[slot] = Value::reference(Rc::clone(&cell));
            cell
        });
        let v = match tag {
            b'N' => Value::NULL,
            b'b' => match self.int(b';')? {
                0 => Value::FALSE,
                1 => Value::TRUE,
                _ => return Err(Bad::At(start)),
            },
            b'i' => Value::int(self.int(b';')?),
            b'd' => {
                let from = self.at;
                while self.at < self.s.len() && self.s[self.at] != b';' {
                    self.at += 1;
                }
                let t = std::str::from_utf8(&self.s[from..self.at]).map_err(|_| Bad::At(start))?;
                let f = match t {
                    "INF" => f64::INFINITY,
                    "-INF" => f64::NEG_INFINITY,
                    "NAN" => f64::NAN,
                    t => t.parse::<f64>().map_err(|_| Bad::At(start))?,
                };
                self.expect(b';')?;
                Value::float(f)
            }
            b's' => {
                let b = self.bytes()?;
                self.expect(b';')?;
                // (One copy, a short one in the string's own allocation.)
                Value::str(PhpStr::from(b))
            }
            // The object an earlier value is (a value of its own).
            b'r' => {
                let n = self.back_number(start)?;
                match self.values.get(n).map(Value::deref) {
                    Some(o) if o.is_object() => o,
                    _ => return Err(Bad::At(self.at)),
                }
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
                Value::array(a)
            }
            b'E' => {
                let body = self.bytes()?;
                self.expect(b';')?;
                let Some(colon) = body.iter().position(|&c| c == b':') else {
                    ctx.warn(&format!("unserialize(): Invalid enum name '{}' (missing colon)", text(body)));
                    return Err(Bad::At(start));
                };
                let (name, case) = (&body[..colon], &body[colon + 1..]);
                if !valid_class_name(name) {
                    return Err(Bad::At(start));
                }
                let Some(class) = ctx.class_load(name)? else {
                    ctx.warn(&format!("unserialize(): Class '{}' not found", text(name)));
                    return Err(Bad::At(start));
                };
                if class.flags & CLASS_ENUM == 0 {
                    ctx.warn(&format!("unserialize(): Class '{}' is not an enum", text(name)));
                    return Err(Bad::At(start));
                }
                match class.constants.get(case) {
                    Some(c) if c.is_case => ctx.enum_case(&class, case),
                    Some(_) => {
                        ctx.warn(&format!("unserialize(): {}::{} is not an enum case", text(name), text(case)));
                        return Err(Bad::At(self.at));
                    }
                    None => {
                        ctx.warn(&format!("unserialize(): Undefined constant {}::{}", text(name), text(case)));
                        return Err(Bad::At(self.at));
                    }
                }
            }
            b'O' => {
                let name = self.bytes()?;
                self.expect(b':')?;
                let n = self.int(b':')?;
                self.expect(b'{')?;
                let allowed = self.allowed.as_ref().is_none_or(|list| list.iter().any(|c| c.eq_ignore_ascii_case(name)));
                let class = if allowed { object_class(ctx, name).map_err(|t| t.map_or(Bad::At(start), Bad::Threw))? } else { None };
                let (o, incomplete) = match class {
                    Some(c) => (ctx.new_object(&c), false),
                    None => {
                        let c = ctx.class(b"__PHP_Incomplete_Class").ok_or(Bad::At(start))?;
                        let o = ctx.new_object(&c);
                        o.dynamic_mut().insert(Key::from_string(PhpStr::from("__PHP_Incomplete_Class_Name")), Value::str(PhpStr::from(name)));
                        (o, true)
                    }
                };
                // (There before its properties are read: an r: in them
                // names it.)
                match &cell {
                    Some(c) => *c.borrow_mut() = Value::object(Rc::clone(&o)),
                    None => self.values[slot] = Value::object(Rc::clone(&o)),
                }
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
                Value::object(o)
            }
            _ => return Err(Bad::At(start)),
        };
        match cell {
            Some(c) => {
                *c.borrow_mut() = v;
                Ok(Value::reference(c))
            }
            None => {
                self.values[slot] = v.clone();
                Ok(v)
            }
        }
    }

    fn key(&mut self, ctx: &mut dyn Ctx) -> Result<Key, Bad> {
        let at = self.at;
        match (self.s.get(at), self.s.get(at + 1)) {
            (Some(b'i'), Some(b':')) => {
                self.at += 2;
                return Ok(Key::Int(self.int(b';')?));
            }
            (Some(b's'), Some(b':')) => {
                self.at += 2;
                let b = self.bytes()?;
                self.expect(b';')?;
                return Ok(Key::from_string(PhpStr::from(b)));
            }
            // A back-reference is no key (Err after it, as PHP reports).
            (Some(b'r' | b'R'), Some(b':')) => {
                self.at += 2;
                self.back_number(at)?;
                return Err(Bad::At(self.at));
            }
            _ => {}
        }
        let k = self.value(ctx)?;
        self.values.pop();
        if let Some(i) = k.as_int() {
            return Ok(Key::Int(i));
        }
        match k.into_str() {
            Ok(s) => Ok(Key::from_string(s)),
            Err(_) => Err(Bad::At(at)),
        }
    }
}

/// Whether `name` can be a class's (zend_is_valid_class_name).
fn valid_class_name(name: &[u8]) -> bool {
    !name.is_empty() && name.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'\\' || c >= 0x80)
}

/// The class an `O:` entry names, looked up where the reader meets it as
/// php_var_unserialize does: declared, else autoloaded, else declared by
/// unserialize_callback_func; None for __PHP_Incomplete_Class. Err(None):
/// a name no class can have.
fn object_class(ctx: &mut dyn Ctx, name: &[u8]) -> Result<Option<Rc<Class>>, Option<Throw>> {
    if let Some(c) = ctx.class(name) {
        return Ok(Some(c));
    }
    if !valid_class_name(name) {
        return Err(None);
    }
    if let Some(c) = ctx.class_load(name)? {
        return Ok(Some(c));
    }
    let func = match ctx.ini_get(b"unserialize_callback_func").and_then(|v| v.into_str().ok()) {
        Some(f) if !f.is_empty() => f,
        _ => return Ok(None),
    };
    let callback = Value::str(func.clone());
    if !func.as_bytes().windows(2).any(|w| w == b"::") && !ctx.is_callable(&callback) {
        let f = text(func.as_bytes());
        return Err(Some(ctx.throw("Error", format!("Invalid callback {f}, function \"{f}\" not found or invalid function name"))));
    }
    ctx.call(&callback, &mut [Value::str(PhpStr::from(name))])?;
    // The callback may have declared it.
    let class = ctx.class_load(name)?;
    if class.is_none() {
        ctx.warn(&format!("unserialize(): Function {}() hasn't defined the class it was called for", text(func.as_bytes())));
    }
    Ok(class)
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
        Some(i) => o.slots()[i].set(v),
        None => {
            o.dynamic_mut().insert(Key::from_string(PhpStr::from(name)), v);
        }
    }
}

/// unserialize(string $data, array $options = [])
fn unserialize(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let data = arg(args, 0).to_php_string().unwrap_or_else(PhpStr::empty);
    let options = arg(args, 1);
    let allowed = match options.as_array() {
        Some(o) => match o.get(&Key::from_string(PhpStr::from("allowed_classes"))).map(Value::deref) {
            Some(v) if v.as_bool() == Some(false) => Some(Vec::new()),
            Some(v) if v.is_array() => Some(v.as_array().expect("an array").values().map(|v| v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec()).collect()),
            _ => None,
        },
        None => None,
    };
    if data.is_empty() {
        return Ok(Value::FALSE);
    }
    let mut r = Reader::new(data.as_bytes(), allowed);
    match r.read(ctx) {
        Ok(v) => {
            r.wake(ctx)?;
            Ok(v)
        }
        Err(Bad::At(at)) => {
            ctx.warn(&format!("unserialize(): Error at offset {at} of {} bytes", data.len()));
            Ok(Value::FALSE)
        }
        Err(Bad::Threw(t)) => Err(t),
    }
}
