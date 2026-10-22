//! Comparison operations
//!
//! Implements PHP comparison operations following Zend semantics.
//!
//! ## PHP Semantics
//!
//! PHP supports two types of equality:
//! - **Loose equality** (`==`): Compares after type juggling
//! - **Strict equality** (`===`): Compares types and values
//!
//! Type juggling rules for comparisons:
//! - Numeric strings compared as numbers
//! - Boolean comparisons convert to bool first
//! - null is less than any value except null itself
//! - Arrays compared by length, then key-by-key
//!
//! ## Operations
//!
//! - **Equal**: `$a == $b` - Loose equality
//! - **NotEqual**: `$a != $b` - Loose inequality
//! - **Identical**: `$a === $b` - Strict equality (type + value)
//! - **NotIdentical**: `$a !== $b` - Strict inequality
//! - **LessThan**: `$a < $b` - Less than comparison
//! - **LessOrEqual**: `$a <= $b` - Less or equal
//! - **GreaterThan**: `$a > $b` - Greater than
//! - **GreaterOrEqual**: `$a >= $b` - Greater or equal
//! - **Spaceship**: `$a <=> $b` - Three-way comparison (-1, 0, 1)
//!
//! ## Performance
//!
//! All comparison operations are O(1) for primitive types.
//! Array/Object comparisons can be O(n) where n is the size.
//!
//! ## References
//!
//! - Zend: `$PHP_SRC_PATH/Zend/zend_operators.c` - compare_function
//! - PHP Manual: https://www.php.net/manual/en/language.operators.comparison.php

use crate::core::value::{Handle, Val};
use crate::vm::engine::{VM, VmError};
use std::cmp::Ordering;

/// Result of a comparison that cannot order its operands (Zend's ZEND_UNCOMPARABLE).
const UNCOMPARABLE: i64 = 1;

/// Arrays and objects nested deeper than this are treated as uncomparable.
const MAX_DEPTH: usize = 256;

impl VM {
    /// Execute Equal operation: $result = $left == $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_equal_function
    #[inline]
    pub(crate) fn exec_equal(&mut self) -> Result<(), VmError> {
        self.binary_cmp_handles(|vm, a, b| vm.loose_equals(a, b))
    }

    /// Execute NotEqual operation: $result = $left != $right
    #[inline]
    pub(crate) fn exec_not_equal(&mut self) -> Result<(), VmError> {
        self.binary_cmp_handles(|vm, a, b| !vm.loose_equals(a, b))
    }

    /// Execute Identical operation: $result = $left === $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - zend_is_identical
    #[inline]
    pub(crate) fn exec_identical(&mut self) -> Result<(), VmError> {
        self.binary_cmp_handles(|vm, a, b| vm.strict_equals(a, b))
    }

    /// Execute NotIdentical operation: $result = $left !== $right
    #[inline]
    pub(crate) fn exec_not_identical(&mut self) -> Result<(), VmError> {
        self.binary_cmp_handles(|vm, a, b| !vm.strict_equals(a, b))
    }

    /// Execute LessThan operation: $result = $left < $right
    #[inline]
    pub(crate) fn exec_less_than(&mut self) -> Result<(), VmError> {
        self.binary_cmp_handles(|vm, a, b| vm.php_compare(a, b) < 0)
    }

    /// Execute LessThanOrEqual operation: $result = $left <= $right
    #[inline]
    pub(crate) fn exec_less_than_or_equal(&mut self) -> Result<(), VmError> {
        self.binary_cmp_handles(|vm, a, b| vm.php_compare(a, b) <= 0)
    }

    /// Execute GreaterThan operation: $result = $left > $right
    /// Zend evaluates `a > b` as `b < a`, which matters for uncomparable operands.
    #[inline]
    pub(crate) fn exec_greater_than(&mut self) -> Result<(), VmError> {
        self.binary_cmp_handles(|vm, a, b| vm.php_compare(b, a) < 0)
    }

    /// Execute GreaterThanOrEqual operation: $result = $left >= $right
    #[inline]
    pub(crate) fn exec_greater_than_or_equal(&mut self) -> Result<(), VmError> {
        self.binary_cmp_handles(|vm, a, b| vm.php_compare(b, a) <= 0)
    }

    /// Execute Spaceship operation: $result = $left <=> $right
    #[inline]
    pub(crate) fn exec_spaceship(&mut self) -> Result<(), VmError> {
        let (a_handle, b_handle) = self.pop_binary_operands()?;
        let result = self.php_compare(a_handle, b_handle);
        let result_handle = self.arena.alloc(Val::Int(result));
        self.operand_stack.push(result_handle);
        Ok(())
    }

    fn binary_cmp_handles(
        &mut self,
        op: impl Fn(&VM, Handle, Handle) -> bool,
    ) -> Result<(), VmError> {
        let (a_handle, b_handle) = self.pop_binary_operands()?;
        let res = op(self, a_handle, b_handle);
        let res_handle = self.arena.alloc(Val::Bool(res));
        self.operand_stack.push(res_handle);
        Ok(())
    }

    /// PHP `==`.
    pub fn loose_equals(&self, a: Handle, b: Handle) -> bool {
        self.compare_at(a, b, 0) == 0
    }

    /// PHP `===`: same type and value; arrays element-wise in order, objects by instance.
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - zend_is_identical
    pub fn strict_equals(&self, a: Handle, b: Handle) -> bool {
        self.identical_at(a, b, 0)
    }

    /// PHP `<=>`: -1, 0 or 1.
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - zend_compare
    pub fn php_compare(&self, a: Handle, b: Handle) -> i64 {
        self.compare_at(a, b, 0)
    }

    fn identical_at(&self, a: Handle, b: Handle, depth: usize) -> bool {
        let (x, y) = (&self.arena.get(a).value, &self.arena.get(b).value);
        match (x, y) {
            (Val::Array(x), Val::Array(y)) => {
                depth < MAX_DEPTH
                    && x.map.len() == y.map.len()
                    && x.map.iter().zip(y.map.iter()).all(|((kx, vx), (ky, vy))| {
                        kx == ky && self.identical_at(*vx, *vy, depth + 1)
                    })
            }
            (Val::Object(x), Val::Object(y)) => x == y,
            _ => x == y,
        }
    }

    fn compare_at(&self, a: Handle, b: Handle, depth: usize) -> i64 {
        let (x, y) = (&self.arena.get(a).value, &self.arena.get(b).value);
        match (x, y) {
            (Val::Array(x), Val::Array(y)) => {
                if depth >= MAX_DEPTH {
                    return UNCOMPARABLE;
                }
                if x.map.len() != y.map.len() {
                    return threeway(x.map.len(), y.map.len());
                }
                for (key, vx) in &x.map {
                    let Some(vy) = y.map.get(key) else {
                        return UNCOMPARABLE;
                    };
                    let c = self.compare_at(*vx, *vy, depth + 1);
                    if c != 0 {
                        return c;
                    }
                }
                0
            }
            (Val::Object(x), Val::Object(y)) => {
                if x == y {
                    return 0;
                }
                let (Val::ObjPayload(ox), Val::ObjPayload(oy)) =
                    (&self.arena.get(*x).value, &self.arena.get(*y).value)
                else {
                    return UNCOMPARABLE;
                };
                if ox.class != oy.class || depth >= MAX_DEPTH {
                    return UNCOMPARABLE;
                }
                if ox.properties.len() != oy.properties.len() {
                    return threeway(ox.properties.len(), oy.properties.len());
                }
                for (name, vx) in &ox.properties {
                    let Some(vy) = oy.properties.get(name) else {
                        return UNCOMPARABLE;
                    };
                    let c = self.compare_at(*vx, *vy, depth + 1);
                    if c != 0 {
                        return c;
                    }
                }
                0
            }
            _ => compare_scalars(x, y),
        }
    }
}

fn threeway<T: PartialOrd>(a: T, b: T) -> i64 {
    match a.partial_cmp(&b) {
        Some(Ordering::Less) => -1,
        Some(Ordering::Equal) => 0,
        // Greater, or NaN involved
        _ => 1,
    }
}

fn compare_numbers(a: &Val, b: &Val) -> i64 {
    match (a, b) {
        (Val::Int(x), Val::Int(y)) => threeway(x, y),
        _ => threeway(a.to_float(), b.to_float()),
    }
}

/// zend_compare for operands that are not both arrays or both objects.
fn compare_scalars(a: &Val, b: &Val) -> i64 {
    use Val::*;
    match (a, b) {
        (Int(_) | Float(_), Int(_) | Float(_)) => compare_numbers(a, b),
        (String(x), String(y)) => {
            if x == y {
                return 0;
            }
            match (Val::numeric_string(x), Val::numeric_string(y)) {
                (Some(nx), Some(ny)) => compare_numbers(&nx, &ny),
                _ => threeway(x.as_slice(), y.as_slice()),
            }
        }
        (Null, String(s)) => -i64::from(!s.is_empty()),
        (String(s), Null) => i64::from(!s.is_empty()),
        // PHP 8: a number equals a string only when the string is numeric;
        // otherwise the number is compared as a string.
        (Float(f), String(_)) | (String(_), Float(f)) if f.is_nan() => UNCOMPARABLE,
        (Int(_) | Float(_), String(s)) => match Val::numeric_string(s) {
            Some(n) => compare_numbers(a, &n),
            None => threeway(a.to_php_string_bytes().as_slice(), s.as_slice()),
        },
        (String(s), Int(_) | Float(_)) => match Val::numeric_string(s) {
            Some(n) => compare_numbers(&n, b),
            None => threeway(s.as_slice(), b.to_php_string_bytes().as_slice()),
        },
        (Null | Bool(false), _) => -i64::from(b.to_bool()),
        (_, Null | Bool(false)) => i64::from(a.to_bool()),
        (Bool(true), _) => i64::from(!b.to_bool()),
        (_, Bool(true)) => -i64::from(!a.to_bool()),
        // An object that is not a number is greater than any string or array.
        (Object(_), Int(_) | Float(_)) => compare_numbers(&Int(1), b),
        (Int(_) | Float(_), Object(_)) => compare_numbers(a, &Int(1)),
        (Object(_), _) => 1,
        (_, Object(_)) => -1,
        (Array(_) | ConstArray(_), _) => 1,
        (_, Array(_) | ConstArray(_)) => -1,
        _ if a == b => 0,
        _ => UNCOMPARABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::context::EngineContext;
    use std::sync::Arc;

    #[test]
    fn test_equal_integers() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(42));
        let right = vm.arena.alloc(Val::Int(42));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_equal().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_equal_with_type_juggling() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(42));
        let right = vm.arena.alloc(Val::String(b"42".to_vec().into()));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_equal().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_not_equal_different_values() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(10));
        let right = vm.arena.alloc(Val::Int(20));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_not_equal().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_identical_same_type_and_value() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(42));
        let right = vm.arena.alloc(Val::Int(42));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_identical().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_identical_different_types() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(42));
        let right = vm.arena.alloc(Val::String(b"42".to_vec().into()));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_identical().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(false)));
    }

    #[test]
    fn test_not_identical_different_types() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(42));
        let right = vm.arena.alloc(Val::Float(42.0));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_not_identical().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_less_than_integers() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(10));
        let right = vm.arena.alloc(Val::Int(20));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_less_than().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_less_than_or_equal() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(20));
        let right = vm.arena.alloc(Val::Int(20));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_less_than_or_equal().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_greater_than_floats() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Float(3.14));
        let right = vm.arena.alloc(Val::Float(2.71));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_greater_than().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_greater_than_or_equal() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(50));
        let right = vm.arena.alloc(Val::Int(50));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_greater_than_or_equal().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_spaceship_less_than() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(5));
        let right = vm.arena.alloc(Val::Int(10));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_spaceship().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Int(-1)));
    }

    #[test]
    fn test_spaceship_equal() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(42));
        let right = vm.arena.alloc(Val::Int(42));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_spaceship().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Int(0)));
    }

    #[test]
    fn test_spaceship_greater_than() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(100));
        let right = vm.arena.alloc(Val::Int(50));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_spaceship().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Int(1)));
    }

    #[test]
    fn test_equal_null_comparisons() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        // null == null should be true
        let left = vm.arena.alloc(Val::Null);
        let right = vm.arena.alloc(Val::Null);

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_equal().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_equal_null_with_false() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        // null == false should be true (both falsy)
        let left = vm.arena.alloc(Val::Null);
        let right = vm.arena.alloc(Val::Bool(false));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_equal().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Bool(true)));
    }

    #[test]
    fn test_spaceship_string_comparison() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::String(b"apple".to_vec().into()));
        let right = vm.arena.alloc(Val::String(b"banana".to_vec().into()));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_spaceship().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        // "apple" < "banana" lexicographically
        assert!(matches!(result_val.value, Val::Int(-1)));
    }
}
