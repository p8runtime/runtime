//! References into arrays (`$a = &$b['k']`, `$b[] = &$a`, an element
//! passed by reference, `foreach ... as &$v`) and `static` variables.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_execute.c (zend_fetch_dimension_address_W,
//! ZEND_ASSIGN_REF, ZEND_FE_FETCH_RW, ZEND_BIND_STATIC)

use crate::Context;
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::Throw;
use p8_runtime::value::{Kind, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

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
        self.walk(root, path, append, At::Set(Value::reference(cell))).map(|_| ())
    }

    fn walk(&mut self, target: &mut Value, path: &[Key], append: bool, at: At) -> Result<Option<Cell>, Throw> {
        if target.is_null_or_undef() {
            *target = Value::array(PhpArray::new());
        }
        let arr = match target.kind() {
            Kind::Array => target.array_mut().expect("an array"),
            Kind::Ref => {
                let cell = target.as_reference().expect("a reference");
                let mut inner = std::mem::take(&mut *cell.borrow_mut());
                let r = self.walk(&mut inner, path, append, at);
                *cell.borrow_mut() = inner;
                return r;
            }
            Kind::Str => return Err(self.throw("Error", "Cannot create references to/from string offsets")),
            Kind::Object => return Err(self.throw("Error", "Cannot use object as array")),
            _ => return Err(self.throw("Error", "Cannot use a scalar value as an array")),
        };
        match path.split_first() {
            None => {
                // `[]`: a new element.
                let (value, cell) = match at {
                    At::Ref => {
                        let cell = Rc::new(RefCell::new(Value::NULL));
                        (Value::reference(Rc::clone(&cell)), Some(cell))
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
                        arr.insert(key.clone(), Value::NULL);
                    }
                    let slot = arr.get_mut(key).expect("just inserted");
                    Ok(Some(make_ref(slot)))
                }
            },
            Some((key, rest)) => {
                if !arr.contains_key(key) {
                    arr.insert(key.clone(), Value::NULL);
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
    pub(crate) fn static_var(&mut self, func: &Arc<p8_bytecode::Function>, index: u16) -> (Cell, bool) {
        let statics = &mut self.statics.entry(Arc::as_ptr(func) as usize).or_insert_with(|| crate::isolate::FunctionStatics::new(func)).cells;
        match statics.get(&index) {
            Some(cell) => (Rc::clone(cell), false),
            None => {
                let cell = Rc::new(RefCell::new(Value::NULL));
                statics.insert(index, Rc::clone(&cell));
                (cell, true)
            }
        }
    }
}

/// A value in place made a reference (if it is not one): its cell.
pub(crate) fn make_ref(slot: &mut Value) -> Cell {
    if let Some(cell) = slot.as_reference() {
        return Rc::clone(cell);
    }
    let value = std::mem::take(slot);
    let value = if value.is_undef() { Value::NULL } else { value };
    let cell = Rc::new(RefCell::new(value));
    *slot = Value::reference(Rc::clone(&cell));
    cell
}

/// foreach by reference: the element at position `pos` or after of the
/// array in `cell`, made a reference: (its position, key, cell).
pub(crate) fn next_ref(cell: &Cell, pos: usize) -> Option<(usize, Key, Cell)> {
    let mut value = cell.borrow_mut();
    let (p, key) = value.as_array()?.entry_at(pos).map(|(p, k, _)| (p, k))?;
    let slot = value.array_mut().expect("an array").get_mut(&key).expect("the element just found");
    Some((p, key, make_ref(slot)))
}
