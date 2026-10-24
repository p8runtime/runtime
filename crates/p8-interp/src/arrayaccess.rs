//! `$o[k]` on objects: ArrayAccess's offsetGet, offsetSet, offsetExists
//! and offsetUnset, for reads, writes (`[]` too), compound assignments,
//! ++/--, isset, ?? and unset. A path that only goes through arrays
//! never comes here.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_interfaces.c (zend_user_it_*),
//! zend_execute.c (zend_fetch_dimension_address, "Indirect modification")

use crate::Context;
use crate::classes::Code;
use p8_runtime::array::Key;
use p8_runtime::ext::Throw;
use p8_runtime::object::Object;
use p8_runtime::ops::BinOp;
use p8_runtime::value::Value;
use std::rc::Rc;

/// Whether the containers on the path `root[k0]...[kn-1]` reach an
/// object (the root itself, or an array element on the way).
pub(crate) fn path_has_object(root: &Value, keys: &[Value], append: bool) -> bool {
    let mut cur = root.deref();
    // The containers written into: root, root[k0], ... (and the last
    // one's element too when appending).
    let walk = if append { keys.len() } else { keys.len().saturating_sub(1) };
    for k in keys.iter().take(walk) {
        match &cur {
            Value::Object(_) => return true,
            Value::Array(a) => match Key::from_value(k).ok().and_then(|(k, _)| a.get(&k).map(Value::deref)) {
                Some(v) => cur = v,
                None => return false,
            },
            _ => return false,
        }
    }
    matches!(cur, Value::Object(_))
}

/// What a dimension operation does at the end of its path.
pub(crate) enum DimOp {
    Set(Value),
    Op(BinOp, Value),
    IncDec { inc: bool, post: bool },
    Unset,
}

impl Context<'_> {
    /// Call ArrayAccess method `name` of `o` with `args`.
    pub(crate) fn aa_call(&mut self, regs: &mut Vec<Value>, o: &Rc<Object>, name: &str, args: &mut [Value]) -> Result<Value, Throw> {
        // An internal class that reads dimensions without being ArrayAccess
        // (DOMNodeList: PHP's read_dimension/has_dimension handlers): its
        // native `$[]::get` and `$[]::has`. Writes stay errors.
        if !o.class.is_a(b"arrayaccess") {
            let handler: &[u8] = match name {
                "offsetGet" => b"$[]::get",
                "offsetExists" => b"$[]::has",
                "offsetSet" => b"$[]::set",
                "offsetUnset" => b"$[]::unset",
                _ => b"",
            };
            if let Some(native) = o.class.method(handler).and_then(|m| m.native) {
                std::mem::swap(&mut self.isolate.regs, regs);
                let r = self.run_native(native, Some(o), args);
                std::mem::swap(&mut self.isolate.regs, regs);
                return r;
            }
            return Err(self.throw("Error", format!("Cannot use object of type {} as array", String::from_utf8_lossy(o.class.name.as_bytes()))));
        }
        let m = self.method_frame(&Value::Object(Rc::clone(o)), name.to_ascii_lowercase().as_bytes(), name.as_bytes())?;
        match m.func {
            Code::Native(native) => {
                let this = m.this.clone();
                std::mem::swap(&mut self.isolate.regs, regs);
                let r = self.run_native(native, this.as_ref(), args);
                std::mem::swap(&mut self.isolate.regs, regs);
                r
            }
            Code::User(_) => self.run_nested(regs, m, args),
        }
    }

    /// `$o[k]` (quiet: for isset/??, offsetExists asked first).
    pub(crate) fn aa_get(&mut self, regs: &mut Vec<Value>, o: &Rc<Object>, key: Value, quiet: bool) -> Result<Value, Throw> {
        if quiet && !self.aa_call(regs, o, "offsetExists", &mut [key.clone()])?.to_bool() {
            return Ok(Value::Null);
        }
        self.aa_call(regs, o, "offsetGet", &mut [key])
    }

    /// A write (or op=, ++/--, unset) down a path that reaches an object;
    /// the expression's value.
    pub(crate) fn aa_write(&mut self, regs: &mut Vec<Value>, root: &Value, keys: &[Value], append: bool, op: DimOp) -> Result<Value, Throw> {
        let mut cur = root.deref();
        let walk = if append { keys.len() } else { keys.len() - 1 };
        for k in &keys[..walk] {
            cur = match cur {
                Value::Object(o) => {
                    let got = self.aa_call(regs, &o, "offsetGet", &mut [k.clone()])?;
                    match got.deref() {
                        v @ Value::Object(_) => v,
                        _ => return self.no_effect(&o, op),
                    }
                }
                Value::Array(a) => match Key::from_value(k).ok().and_then(|(k, _)| a.get(&k).map(Value::deref)) {
                    Some(v) => v,
                    None => return Ok(Value::Null),
                },
                _ => return Err(self.throw("Error", "Cannot use a scalar value as an array")),
            };
        }
        let Value::Object(o) = cur else {
            return Err(self.throw("Error", "Cannot use a scalar value as an array"));
        };
        let key = if append { Value::Null } else { keys[keys.len() - 1].clone() };
        match op {
            DimOp::Set(v) => {
                self.aa_call(regs, &o, "offsetSet", &mut [key, v.clone()])?;
                Ok(v)
            }
            DimOp::Op(op, v) => {
                let old = self.aa_call(regs, &o, "offsetGet", &mut [key.clone()])?;
                let r = p8_runtime::ops::binary(op, &old.deref(), &v, &mut self.diags);
                self.report_diags();
                let new = r.map_err(|e| self.op_error(e))?;
                self.aa_call(regs, &o, "offsetSet", &mut [key, new.clone()])?;
                Ok(new)
            }
            // ++/-- work on what offsetGet returned (a copy): PHP says so,
            // and offsetSet is not called.
            DimOp::IncDec { inc, post } => {
                let old = self.aa_call(regs, &o, "offsetGet", &mut [key])?.deref();
                let message = format!("Indirect modification of overloaded element of {} has no effect", String::from_utf8_lossy(o.class.name.as_bytes()));
                self.notice(&message);
                let r = if inc { p8_runtime::ops::increment(&old, &mut self.diags) } else { p8_runtime::ops::decrement(&old, &mut self.diags) };
                self.report_diags();
                let new = r.map_err(|e| self.op_error(e))?;
                Ok(if post { old } else { new })
            }
            DimOp::Unset => {
                self.aa_call(regs, &o, "offsetUnset", &mut [key])?;
                Ok(Value::Null)
            }
        }
    }

    /// A nested write through an element offsetGet returned (not an
    /// object): it changes nothing.
    fn no_effect(&mut self, o: &Object, op: DimOp) -> Result<Value, Throw> {
        if !matches!(op, DimOp::Unset) {
            let message = format!("Indirect modification of overloaded element of {} has no effect", String::from_utf8_lossy(o.class.name.as_bytes()));
            self.notice(&message);
        }
        Ok(match op {
            DimOp::Set(v) | DimOp::Op(_, v) => v,
            _ => Value::Null,
        })
    }

    /// `isset($o[k]...)` down a path that reaches an object: offsetExists
    /// decides at an object (then its offsetGet goes on).
    pub(crate) fn aa_isset(&mut self, regs: &mut Vec<Value>, root: &Value, keys: &[Value]) -> Result<bool, Throw> {
        let mut cur = root.deref();
        for (i, k) in keys.iter().enumerate() {
            let last = i + 1 == keys.len();
            cur = match cur {
                Value::Object(o) => {
                    if !self.aa_call(regs, &o, "offsetExists", &mut [k.clone()])?.to_bool() {
                        return Ok(false);
                    }
                    if last {
                        return Ok(true);
                    }
                    self.aa_call(regs, &o, "offsetGet", &mut [k.clone()])?.deref()
                }
                Value::Array(a) => match Key::from_value(k).ok().and_then(|(k, _)| a.get(&k).map(Value::deref)) {
                    Some(v) => v,
                    None => return Ok(false),
                },
                _ => return Ok(false),
            };
        }
        Ok(!matches!(cur, Value::Null | Value::Undef))
    }
}
