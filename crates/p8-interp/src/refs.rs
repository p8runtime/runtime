//! References into arrays (`$a = &$b['k']`, `$b[] = &$a`, an element
//! passed by reference, `foreach ... as &$v`) and `static` variables.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_execute.c (zend_fetch_dimension_address_W,
//! ZEND_ASSIGN_REF, ZEND_FE_FETCH_RW, ZEND_BIND_STATIC)

use crate::Context;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Throw;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) type Cell = Rc<RefCell<Value>>;

/// What to do at the end of a path.
enum At {
    /// Make the element a reference (if it is not one) and give its cell.
    Ref,
    /// Put this value there, replacing even a reference.
    Set(Value),
}

impl Context<'_> {
    /// The cell of `root[k1]..[kn]` (a last `[]` with `append`), the
    /// element made a reference; arrays are made along the way, as a
    /// write does.
    pub(crate) fn dim_ref(&mut self, root: &mut Value, path: &[Key], append: bool) -> Result<Cell, Throw> {
        match self.walk(root, path, append, At::Ref)? {
            Some(cell) => Ok(cell),
            None => unreachable!("a reference is made"),
        }
    }

    /// `root[k1]..[kn] = &cell`: the element is the reference itself.
    pub(crate) fn dim_set_ref(&mut self, root: &mut Value, path: &[Key], append: bool, cell: Cell) -> Result<(), Throw> {
        self.walk(root, path, append, At::Set(Value::Ref(cell))).map(|_| ())
    }

    fn walk(&mut self, target: &mut Value, path: &[Key], append: bool, at: At) -> Result<Option<Cell>, Throw> {
        if matches!(target, Value::Null | Value::Undef) {
            *target = Value::Array(Rc::new(PhpArray::new()));
        }
        let arr = match target {
            Value::Array(a) => Rc::make_mut(a),
            Value::Ref(cell) => {
                let mut inner = std::mem::take(&mut *cell.borrow_mut());
                let r = self.walk(&mut inner, path, append, at);
                *cell.borrow_mut() = inner;
                return r;
            }
            Value::Str(_) => return Err(self.throw("Error", "Cannot create references to/from string offsets")),
            Value::Object(_) => return Err(self.throw("Error", "Cannot use object as array")),
            _ => return Err(self.throw("Error", "Cannot use a scalar value as an array")),
        };
        match path.split_first() {
            None => {
                // `[]`: a new element.
                let (value, cell) = match at {
                    At::Ref => {
                        let cell = Rc::new(RefCell::new(Value::Null));
                        (Value::Ref(Rc::clone(&cell)), Some(cell))
                    }
                    At::Set(v) => (v, None),
                };
                if arr.push(value).is_err() {
                    return Err(self.throw("Error", "Cannot add element to the array as the next element is already occupied"));
                }
                Ok(cell)
            }
            Some((key, rest)) if rest.is_empty() && !append => match at {
                At::Set(v) => {
                    arr.insert(key.clone(), v);
                    Ok(None)
                }
                At::Ref => {
                    if !arr.contains_key(key) {
                        arr.insert(key.clone(), Value::Null);
                    }
                    let slot = arr.get_mut(key).expect("just inserted");
                    Ok(Some(make_ref(slot)))
                }
            },
            Some((key, rest)) => {
                if !arr.contains_key(key) {
                    arr.insert(key.clone(), Value::Null);
                }
                let mut child = std::mem::take(arr.get_mut(key).expect("present"));
                let r = self.walk(&mut child, rest, append, at);
                *arr.get_mut(key).expect("present") = child;
                r
            }
        }
    }

    /// `static $x`: the cell of static `index` of the running function,
    /// and whether it is new (its initializer runs then).
    pub(crate) fn static_var(&mut self, func: &p8_bytecode::Function, index: u16) -> (Cell, bool) {
        let statics = self.statics.entry(func as *const p8_bytecode::Function as usize).or_default();
        match statics.get(&index) {
            Some(cell) => (Rc::clone(cell), false),
            None => {
                let cell = Rc::new(RefCell::new(Value::Null));
                statics.insert(index, Rc::clone(&cell));
                (cell, true)
            }
        }
    }
}

/// A value in place made a reference (if it is not one): its cell.
pub(crate) fn make_ref(slot: &mut Value) -> Cell {
    if let Value::Ref(cell) = slot {
        return Rc::clone(cell);
    }
    let value = match std::mem::take(slot) {
        Value::Undef => Value::Null,
        v => v,
    };
    let cell = Rc::new(RefCell::new(value));
    *slot = Value::Ref(Rc::clone(&cell));
    cell
}

/// foreach by reference: the element at position `pos` or after of the
/// array in `cell`, made a reference: (its position, key, cell).
pub(crate) fn next_ref(cell: &Cell, pos: usize) -> Option<(usize, Key, Cell)> {
    let mut value = cell.borrow_mut();
    let Value::Array(a) = &mut *value else { return None };
    let (p, key) = a.entry_at(pos).map(|(p, k, _)| (p, k))?;
    let slot = Rc::make_mut(a).get_mut(&key).expect("the element just found");
    Some((p, key, make_ref(slot)))
}
