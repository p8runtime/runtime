//! #391 (prototype): every value that holds memory behind ONE pointer
//! type, `Rc<Heap>`, as every zend heap value starts with the same
//! `zend_refcounted` header. Copying such a value is then one increment
//! at a fixed offset with no per-kind dispatch; what the kind costs is a
//! byte read behind the pointer, next to the count.
//!
//! The typed handles the rest of the engine names (`HRc<Object>`,
//! `HRc<PhpArray>`, `HRc<RefCell<Value>>`, a string's `HRc<StrHead>`) are
//! that one `Rc<Heap>` with the kind it is known to hold: dereferencing
//! one tests the kind byte (a compare and a branch never taken).

use crate::array::PhpArray;
use crate::object::Object;
use crate::string::StrHead;
use crate::value::Value;
use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;
use std::rc::{Rc, Weak};

/// What a heap value's pointer points at: its kind, then its body.
/// `repr(u8)`: the kind is a byte at the start of the body, after the
/// `Rc` counts (no niche encoding to decode).
#[repr(u8)]
pub enum Heap {
    Str(StrHead) = 0,
    Array(PhpArray) = 1,
    /// A PHP reference: the shared cell `$a = &$b` makes.
    Ref(RefCell<Value>) = 2,
    /// Boxed: an object's body is 200 bytes, and an enum is as big as its
    /// biggest variant, so unboxed every string's and array's allocation
    /// was 224 bytes (measured: `concat` +11 cycles per iteration, store
    /// forwards from moving the 208-byte body into its `Rc`).
    Object(Box<Object>) = 3,
}

impl fmt::Debug for Heap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Heap::Str(s) => s.fmt(f),
            Heap::Array(a) => a.fmt(f),
            Heap::Ref(c) => f.debug_tuple("Ref").field(c).finish(),
            Heap::Object(o) => write!(f, "Object(#{})", o.id),
        }
    }
}

/// A body a heap value can hold.
pub trait HeapKind: Sized {
    fn wrap(self) -> Heap;
    fn get(h: &Heap) -> Option<&Self>;
    fn get_mut(h: &mut Heap) -> Option<&mut Self>;
    fn take(h: Heap) -> Result<Self, Heap>;
}

macro_rules! kind {
    ($t:ty, $v:ident) => {
        impl HeapKind for $t {
            #[inline(always)]
            fn wrap(self) -> Heap {
                Heap::$v(self)
            }
            #[inline(always)]
            fn get(h: &Heap) -> Option<&Self> {
                match h {
                    Heap::$v(x) => Some(x),
                    _ => None,
                }
            }
            #[inline(always)]
            fn get_mut(h: &mut Heap) -> Option<&mut Self> {
                match h {
                    Heap::$v(x) => Some(x),
                    _ => None,
                }
            }
            #[inline(always)]
            fn take(h: Heap) -> Result<Self, Heap> {
                match h {
                    Heap::$v(x) => Ok(x),
                    h => Err(h),
                }
            }
        }
    };
}
kind!(StrHead, Str);
kind!(PhpArray, Array);
kind!(RefCell<Value>, Ref);

impl HeapKind for Object {
    #[inline(always)]
    fn wrap(self) -> Heap {
        Heap::Object(Box::new(self))
    }
    #[inline(always)]
    fn get(h: &Heap) -> Option<&Self> {
        match h {
            Heap::Object(x) => Some(x),
            _ => None,
        }
    }
    #[inline(always)]
    fn get_mut(h: &mut Heap) -> Option<&mut Self> {
        match h {
            Heap::Object(x) => Some(x),
            _ => None,
        }
    }
    #[inline(always)]
    fn take(h: Heap) -> Result<Self, Heap> {
        match h {
            Heap::Object(x) => Ok(*x),
            h => Err(h),
        }
    }
}

#[cold]
#[inline(never)]
fn wrong_kind() -> ! {
    panic!("a typed heap handle over another kind of heap value")
}

/// A shared handle on a heap value known to hold a `T`: the one
/// `Rc<Heap>`, typed.
pub struct HRc<T: HeapKind>(Rc<Heap>, PhantomData<T>);

impl<T: HeapKind> HRc<T> {
    #[inline]
    pub fn new(t: T) -> HRc<T> {
        HRc(Rc::new(t.wrap()), PhantomData)
    }

    /// The handle of a heap value, if it holds a `T`.
    #[inline(always)]
    pub fn from_heap(rc: Rc<Heap>) -> Result<HRc<T>, Rc<Heap>> {
        if T::get(&rc).is_some() { Ok(HRc(rc, PhantomData)) } else { Err(rc) }
    }

    /// The untyped handle (shared).
    #[inline(always)]
    pub fn heap(this: &HRc<T>) -> &Rc<Heap> {
        &this.0
    }

    /// The untyped handle, for writing in place: the caller keeps the
    /// kind (a string's writer replaces a string with a string).
    #[inline(always)]
    pub(crate) fn heap_mut(this: &mut HRc<T>) -> &mut Rc<Heap> {
        &mut this.0
    }

    #[inline(always)]
    pub fn into_heap(this: HRc<T>) -> Rc<Heap> {
        this.0
    }

    #[inline(always)]
    pub fn ptr_eq(a: &HRc<T>, b: &HRc<T>) -> bool {
        Rc::ptr_eq(&a.0, &b.0)
    }

    /// The body's address (identity, as `Rc::as_ptr`).
    #[inline(always)]
    pub fn as_ptr(this: &HRc<T>) -> *const T {
        &**this as *const T
    }

    #[inline(always)]
    pub fn strong_count(this: &HRc<T>) -> usize {
        Rc::strong_count(&this.0)
    }

    #[inline]
    pub fn get_mut(this: &mut HRc<T>) -> Option<&mut T> {
        Rc::get_mut(&mut this.0).map(|h| T::get_mut(h).unwrap_or_else(|| wrong_kind()))
    }

    #[inline]
    pub fn downgrade(this: &HRc<T>) -> HWeak<T> {
        HWeak(Rc::downgrade(&this.0), PhantomData)
    }

    pub fn try_unwrap(this: HRc<T>) -> Result<T, HRc<T>> {
        match Rc::try_unwrap(this.0) {
            Ok(h) => Ok(T::take(h).unwrap_or_else(|_| wrong_kind())),
            Err(rc) => Err(HRc(rc, PhantomData)),
        }
    }
}

impl<T: HeapKind + Clone> HRc<T> {
    /// The body, for writing: copied first if shared (`Rc::make_mut`).
    #[inline]
    pub fn make_mut(this: &mut HRc<T>) -> &mut T {
        if Rc::get_mut(&mut this.0).is_none() {
            let copy = T::clone(this);
            this.0 = Rc::new(copy.wrap());
        }
        HRc::get_mut(this).unwrap_or_else(|| wrong_kind())
    }
}

impl<T: HeapKind> Deref for HRc<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        T::get(&self.0).unwrap_or_else(|| wrong_kind())
    }
}

impl<T: HeapKind> Clone for HRc<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        HRc(Rc::clone(&self.0), PhantomData)
    }
}

impl<T: HeapKind> From<T> for HRc<T> {
    #[inline]
    fn from(t: T) -> HRc<T> {
        HRc::new(t)
    }
}

impl<T: HeapKind + fmt::Debug> fmt::Debug for HRc<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

/// A weak handle on a heap value holding a `T`.
pub struct HWeak<T: HeapKind>(Weak<Heap>, PhantomData<T>);

impl<T: HeapKind> HWeak<T> {
    pub fn new() -> HWeak<T> {
        HWeak(Weak::new(), PhantomData)
    }

    pub fn upgrade(&self) -> Option<HRc<T>> {
        self.0.upgrade().map(|rc| HRc(rc, PhantomData))
    }

    pub fn strong_count(&self) -> usize {
        self.0.strong_count()
    }
}

impl<T: HeapKind> Default for HWeak<T> {
    fn default() -> Self {
        HWeak::new()
    }
}

impl<T: HeapKind> fmt::Debug for HWeak<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(Weak)")
    }
}

impl<T: HeapKind> Clone for HWeak<T> {
    fn clone(&self) -> Self {
        HWeak(self.0.clone(), PhantomData)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    /// The sizes the one heap type costs: every allocation is the biggest
    /// body's (printed for #391's PR; run with --nocapture).
    #[test]
    fn sizes() {
        println!(
            "Value {} Option<Value> {} Heap {} StrHead {} PhpArray {} RefCell<Value> {} Object {}",
            size_of::<Value>(),
            size_of::<Option<Value>>(),
            size_of::<Heap>(),
            size_of::<StrHead>(),
            size_of::<PhpArray>(),
            size_of::<RefCell<Value>>(),
            size_of::<Object>()
        );
        let s = HRc::new(StrHead::clone(&crate::string::PhpStr::from("x")));
        assert_eq!(HRc::strong_count(&s), 1);
        assert_eq!(s.as_bytes(), b"x");
    }
}
