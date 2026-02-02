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

use crate::core::value::Val;
use crate::vm::engine::{VM, VmError};

impl VM {
    /// Strict equality with struct value semantics
    fn strict_equals(&mut self, a: &Val, b: &Val) -> bool {
        match (a, b) {
            (Val::Struct(a_data), Val::Struct(b_data)) => {
                if a_data.class != b_data.class {
                    return false;
                }
                if let Some(class_def) = self.context.classes.get(&a_data.class) {
                    if class_def.is_enum {
                        let name_sym = self.context.interner.intern(b"name");
                        let Some(a_handle) = a_data.properties.get(&name_sym) else {
                            return false;
                        };
                        let Some(b_handle) = b_data.properties.get(&name_sym) else {
                            return false;
                        };
                        let a_val = self.arena.get(*a_handle).value.clone();
                        let b_val = self.arena.get(*b_handle).value.clone();
                        return self.strict_equals(&a_val, &b_val);
                    }
                }
                if a_data.properties.len() != b_data.properties.len() {
                    return false;
                }
                for (key, a_handle) in a_data.properties.iter() {
                    let Some(b_handle) = b_data.properties.get(key) else {
                        return false;
                    };
                    let a_val = self.arena.get(*a_handle).value.clone();
                    let b_val = self.arena.get(*b_handle).value.clone();
                    if !self.strict_equals(&a_val, &b_val) {
                        return false;
                    }
                }
                true
            }
            (Val::ObjectMap(a_map), Val::ObjectMap(b_map)) => {
                if a_map.map.len() != b_map.map.len() {
                    return false;
                }
                for (key, a_handle) in a_map.map.iter() {
                    let Some(b_handle) = b_map.map.get(key) else {
                        return false;
                    };
                    let a_val = self.arena.get(*a_handle).value.clone();
                    let b_val = self.arena.get(*b_handle).value.clone();
                    if !self.strict_equals(&a_val, &b_val) {
                        return false;
                    }
                }
                true
            }
            _ => a == b,
        }
    }

    fn loose_equals(&mut self, a: &Val, b: &Val) -> bool {
        match (a, b) {
            (Val::Struct(a_data), Val::Struct(b_data)) => {
                if a_data.class != b_data.class {
                    return false;
                }
                if let Some(class_def) = self.context.classes.get(&a_data.class) {
                    if class_def.is_enum {
                        let name_sym = self.context.interner.intern(b"name");
                        let Some(a_handle) = a_data.properties.get(&name_sym) else {
                            return false;
                        };
                        let Some(b_handle) = b_data.properties.get(&name_sym) else {
                            return false;
                        };
                        let a_val = self.arena.get(*a_handle).value.clone();
                        let b_val = self.arena.get(*b_handle).value.clone();
                        return self.loose_equals(&a_val, &b_val);
                    }
                }
                if a_data.properties.len() != b_data.properties.len() {
                    return false;
                }
                for (key, a_handle) in a_data.properties.iter() {
                    let Some(b_handle) = b_data.properties.get(key) else {
                        return false;
                    };
                    let a_val = self.arena.get(*a_handle).value.clone();
                    let b_val = self.arena.get(*b_handle).value.clone();
                    if !self.loose_equals(&a_val, &b_val) {
                        return false;
                    }
                }
                true
            }
            (Val::ObjectMap(a_map), Val::ObjectMap(b_map)) => {
                if a_map.map.len() != b_map.map.len() {
                    return false;
                }
                for (key, a_handle) in a_map.map.iter() {
                    let Some(b_handle) = b_map.map.get(key) else {
                        return false;
                    };
                    let a_val = self.arena.get(*a_handle).value.clone();
                    let b_val = self.arena.get(*b_handle).value.clone();
                    if !self.loose_equals(&a_val, &b_val) {
                        return false;
                    }
                }
                true
            }
            _ => php_loose_equals(a, b),
        }
    }

    /// Execute Equal operation: $result = $left == $right
    /// PHP loose equality with type juggling
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_equal_function
    #[inline]
    pub(crate) fn exec_equal(&mut self) -> Result<(), VmError> {
        let (a_handle, b_handle) = self.pop_binary_operands()?;
        let a_val = self.arena.get(a_handle).value.clone();
        let b_val = self.arena.get(b_handle).value.clone();
        let res = self.loose_equals(&a_val, &b_val);
        let res_handle = self.arena.alloc(Val::Bool(res));
        self.operand_stack.push(res_handle);
        Ok(())
    }

    /// Execute NotEqual operation: $result = $left != $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_not_equal_function
    #[inline]
    pub(crate) fn exec_not_equal(&mut self) -> Result<(), VmError> {
        let (a_handle, b_handle) = self.pop_binary_operands()?;
        let a_val = self.arena.get(a_handle).value.clone();
        let b_val = self.arena.get(b_handle).value.clone();
        let res = !self.loose_equals(&a_val, &b_val);
        let res_handle = self.arena.alloc(Val::Bool(res));
        self.operand_stack.push(res_handle);
        Ok(())
    }

    /// Execute Identical operation: $result = $left === $right
    /// Strict equality (no type juggling)
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_identical_function
    #[inline]
    pub(crate) fn exec_identical(&mut self) -> Result<(), VmError> {
        let (a_handle, b_handle) = self.pop_binary_operands()?;
        let a_val = self.arena.get(a_handle).value.clone();
        let b_val = self.arena.get(b_handle).value.clone();
        let res = self.strict_equals(&a_val, &b_val);
        let res_handle = self.arena.alloc(Val::Bool(res));
        self.operand_stack.push(res_handle);
        Ok(())
    }

    /// Execute NotIdentical operation: $result = $left !== $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_not_identical_function
    #[inline]
    pub(crate) fn exec_not_identical(&mut self) -> Result<(), VmError> {
        let (a_handle, b_handle) = self.pop_binary_operands()?;
        let a_val = self.arena.get(a_handle).value.clone();
        let b_val = self.arena.get(b_handle).value.clone();
        let res = !self.strict_equals(&a_val, &b_val);
        let res_handle = self.arena.alloc(Val::Bool(res));
        self.operand_stack.push(res_handle);
        Ok(())
    }

    /// Execute LessThan operation: $result = $left < $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_smaller_function
    #[inline]
    pub(crate) fn exec_less_than(&mut self) -> Result<(), VmError> {
        self.binary_cmp(|a, b| php_compare(a, b) < 0)
    }

    /// Execute LessThanOrEqual operation: $result = $left <= $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_smaller_or_equal_function
    #[inline]
    pub(crate) fn exec_less_than_or_equal(&mut self) -> Result<(), VmError> {
        self.binary_cmp(|a, b| php_compare(a, b) <= 0)
    }

    /// Execute GreaterThan operation: $result = $left > $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_smaller_function (inverted)
    #[inline]
    pub(crate) fn exec_greater_than(&mut self) -> Result<(), VmError> {
        self.binary_cmp(|a, b| php_compare(a, b) > 0)
    }

    /// Execute GreaterThanOrEqual operation: $result = $left >= $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - is_smaller_or_equal_function (inverted)
    #[inline]
    pub(crate) fn exec_greater_than_or_equal(&mut self) -> Result<(), VmError> {
        self.binary_cmp(|a, b| php_compare(a, b) >= 0)
    }

    /// Execute Spaceship operation: $result = $left <=> $right
    /// Returns -1, 0, or 1
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - compare_function
    #[inline]
    pub(crate) fn exec_spaceship(&mut self) -> Result<(), VmError> {
        let (a_handle, b_handle) = self.pop_binary_operands()?;

        let a_val = &self.arena.get(a_handle).value;
        let b_val = &self.arena.get(b_handle).value;

        let result = php_compare(a_val, b_val);
        let result_handle = self.arena.alloc(Val::Int(result));
        self.operand_stack.push(result_handle);
        Ok(())
    }
}

/// PHP loose equality (==) with type juggling
/// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - zend_compare
fn php_loose_equals(a: &Val, b: &Val) -> bool {
    match (a, b) {
        // Same types - direct comparison
        (Val::Null, Val::Null) => true,
        (Val::Bool(x), Val::Bool(y)) => x == y,
        (Val::Int(x), Val::Int(y)) => x == y,
        (Val::Float(x), Val::Float(y)) => x == y,
        (Val::String(x), Val::String(y)) => x == y,
        (Val::Struct(x), Val::Struct(y)) => x == y,

        // Numeric comparisons with type juggling
        (Val::Int(x), Val::Float(y)) => *x as f64 == *y,
        (Val::Float(x), Val::Int(y)) => *x == *y as f64,

        // Bool comparisons (convert to bool)
        (Val::Bool(x), _) => *x == b.to_bool(),
        (_, Val::Bool(y)) => a.to_bool() == *y,

        // Null comparisons
        (Val::Null, _) => !b.to_bool(),
        (_, Val::Null) => !a.to_bool(),

        // String/numeric comparisons
        (Val::String(_), Val::Int(_))
        | (Val::String(_), Val::Float(_))
        | (Val::Int(_), Val::String(_))
        | (Val::Float(_), Val::String(_)) => {
            // Convert both to numeric and compare
            let a_num = a.to_float();
            let b_num = b.to_float();
            a_num == b_num
        }

        _ => false,
    }
}

/// PHP comparison function - returns -1, 0, or 1
/// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - compare_function
fn php_compare(a: &Val, b: &Val) -> i64 {
    match (a, b) {
        // Integer comparisons
        (Val::Int(x), Val::Int(y)) => {
            if x < y {
                -1
            } else if x > y {
                1
            } else {
                0
            }
        }

        // Float comparisons
        (Val::Float(x), Val::Float(y)) => {
            if x < y {
                -1
            } else if x > y {
                1
            } else {
                0
            }
        }

        // Mixed numeric
        (Val::Int(x), Val::Float(y)) => {
            let xf = *x as f64;
            if xf < *y {
                -1
            } else if xf > *y {
                1
            } else {
                0
            }
        }
        (Val::Float(x), Val::Int(y)) => {
            let yf = *y as f64;
            if x < &yf {
                -1
            } else if x > &yf {
                1
            } else {
                0
            }
        }

        // String comparisons (lexicographic)
        (Val::String(x), Val::String(y)) => {
            if x < y {
                -1
            } else if x > y {
                1
            } else {
                0
            }
        }

        // Bool comparisons
        (Val::Bool(x), Val::Bool(y)) => {
            if x < y {
                -1
            } else if x > y {
                1
            } else {
                0
            }
        }

        // Null comparisons
        (Val::Null, Val::Null) => 0,
        (Val::Null, _) => {
            if b.to_bool() {
                -1
            } else {
                0
            }
        }
        (_, Val::Null) => {
            if a.to_bool() {
                1
            } else {
                0
            }
        }

        // Type juggling for other cases
        _ => {
            let a_num = a.to_float();
            let b_num = b.to_float();
            if a_num < b_num {
                -1
            } else if a_num > b_num {
                1
            } else {
                0
            }
        }
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
