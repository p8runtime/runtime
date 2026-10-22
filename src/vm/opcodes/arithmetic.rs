//! Arithmetic operations
//!
//! Implements PHP arithmetic operations following Zend engine semantics.
//!
//! ## PHP Semantics
//!
//! PHP arithmetic operations perform automatic type juggling:
//! - Numeric strings are converted to integers/floats
//! - Booleans: true=1, false=0
//! - null converts to 0
//! - Arrays/Objects cause type errors or warnings
//!
//! ## Operations
//!
//! - **Add**: `$a + $b` - Addition with type coercion
//! - **Sub**: `$a - $b` - Subtraction
//! - **Mul**: `$a * $b` - Multiplication
//! - **Div**: `$a / $b` - Division (returns float or int)
//! - **Mod**: `$a % $b` - Modulo operation
//! - **Pow**: `$a ** $b` - Exponentiation
//!
//! ## Performance
//!
//! All operations are O(1) after type conversion. Type juggling may
//! allocate new values on the arena.
//!
//! ## References
//!
//! - Zend: `$PHP_SRC_PATH/Zend/zend_operators.c` - arithmetic functions
//! - PHP Manual: https://www.php.net/manual/en/language.operators.arithmetic.php

use crate::core::value::Val;
use crate::vm::engine::{ErrorLevel, VM, VmError};
use std::rc::Rc;

#[derive(Debug, Clone, Copy)]
enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

impl ArithOp {
    fn symbol(self) -> &'static str {
        match self {
            ArithOp::Add => "+",
            ArithOp::Sub => "-",
            ArithOp::Mul => "*",
            ArithOp::Div => "/",
            ArithOp::Mod => "%",
            ArithOp::Pow => "**",
        }
    }
}

/// An arithmetic operand after zendi_smart_str-style conversion.
#[derive(Debug, Clone, Copy)]
enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    fn float(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }

    fn into_val(self) -> Val {
        match self {
            Num::Int(i) => Val::Int(i),
            Num::Float(f) => Val::Float(f),
        }
    }
}

/// `base ** exp` for a non-negative exponent, or None on overflow.
fn int_pow(base: i64, exp: i64) -> Option<i64> {
    u32::try_from(exp).ok().and_then(|e| base.checked_pow(e))
}

impl VM {
    /// PHP's type name for an operand in "Unsupported operand types" errors.
    fn operand_type_name(&self, val: &Val) -> String {
        match val {
            Val::Object(payload) => match &self.arena.get(*payload).value {
                Val::ObjPayload(obj) => String::from_utf8_lossy(
                    self.context.interner.lookup(obj.class).unwrap_or(b"object"),
                )
                .into_owned(),
                _ => "object".into(),
            },
            other => other.type_name().into(),
        }
    }

    /// Convert an operand to a number, or None when PHP throws a TypeError for it.
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - zendi_try_convert_scalar_to_number
    fn arith_operand(&mut self, val: &Val) -> Option<Num> {
        match val {
            Val::Null => Some(Num::Int(0)),
            Val::Bool(b) => Some(Num::Int(i64::from(*b))),
            Val::Int(i) => Some(Num::Int(*i)),
            Val::Float(f) => Some(Num::Float(*f)),
            Val::String(s) => {
                let num = match Val::numeric_string(s) {
                    Some(n) => n,
                    // Leading-numeric ("12abc") works with a warning; anything else throws.
                    None => {
                        let (n, _) = Val::leading_numeric(s)?;
                        self.report_error(ErrorLevel::Warning, "A non-numeric value encountered");
                        n
                    }
                };
                match num {
                    Val::Int(i) => Some(Num::Int(i)),
                    Val::Float(f) => Some(Num::Float(f)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// An operand of `%`, converted to int as zval_get_long does (with the
    /// PHP 8.1 deprecation for floats that lose precision).
    fn mod_operand(&mut self, num: Num) -> i64 {
        match num {
            Num::Int(i) => i,
            Num::Float(f) => {
                if f.fract() != 0.0 && f.is_finite() {
                    let msg = format!(
                        "Implicit conversion from float {} to int loses precision",
                        crate::core::value::php_float_to_string(
                            f,
                            crate::core::value::FloatPrecision::Shortest,
                            'E'
                        )
                    );
                    self.report_error(ErrorLevel::Deprecated, &msg);
                }
                f as i64
            }
        }
    }

    /// Generic binary arithmetic operation
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - add_function, mul_function, ...
    fn binary_arithmetic(&mut self, op: ArithOp) -> Result<(), VmError> {
        let (a_handle, b_handle) = self.pop_binary_operands()?;
        let a_val = self.arena.get(a_handle).value.clone();
        let b_val = self.arena.get(b_handle).value.clone();

        // Array + Array = union
        if let (ArithOp::Add, Val::Array(a_arr), Val::Array(b_arr)) = (op, &a_val, &b_val) {
            let mut result = (**a_arr).clone();
            for (k, v) in b_arr.map.iter() {
                result.map.entry(k.clone()).or_insert(*v);
            }
            self.operand_stack
                .push(self.arena.alloc(Val::Array(Rc::new(result))));
            return Ok(());
        }

        let unsupported = |vm: &mut VM| {
            let msg = format!(
                "Unsupported operand types: {} {} {}",
                vm.operand_type_name(&a_val),
                op.symbol(),
                vm.operand_type_name(&b_val)
            );
            vm.throw_error(b"TypeError", &msg)
        };
        // Both operands are checked before either is converted, as Zend does.
        let numeric_type = |v: &Val| !matches!(v, Val::Array(_) | Val::ConstArray(_) | Val::Object(_) | Val::Resource(_));
        if !numeric_type(&a_val) || !numeric_type(&b_val) {
            return Err(unsupported(self));
        }
        let (Some(a), Some(b)) = (self.arith_operand(&a_val), self.arith_operand(&b_val)) else {
            return Err(unsupported(self));
        };

        let result = match (op, a, b) {
            (ArithOp::Add, Num::Int(x), Num::Int(y)) => x.checked_add(y).map_or(Num::Float(x as f64 + y as f64), Num::Int),
            (ArithOp::Sub, Num::Int(x), Num::Int(y)) => x.checked_sub(y).map_or(Num::Float(x as f64 - y as f64), Num::Int),
            (ArithOp::Mul, Num::Int(x), Num::Int(y)) => x.checked_mul(y).map_or(Num::Float(x as f64 * y as f64), Num::Int),
            (ArithOp::Add, _, _) => Num::Float(a.float() + b.float()),
            (ArithOp::Sub, _, _) => Num::Float(a.float() - b.float()),
            (ArithOp::Mul, _, _) => Num::Float(a.float() * b.float()),
            (ArithOp::Div, _, _) => {
                if b.float() == 0.0 {
                    return Err(self.throw_error(b"DivisionByZeroError", "Division by zero"));
                }
                match (a, b) {
                    (Num::Int(x), Num::Int(y)) if x.checked_rem(y) == Some(0) => {
                        x.checked_div(y).map_or(Num::Float(x as f64 / y as f64), Num::Int)
                    }
                    _ => Num::Float(a.float() / b.float()),
                }
            }
            (ArithOp::Mod, _, _) => {
                let x = self.mod_operand(a);
                let y = self.mod_operand(b);
                if y == 0 {
                    return Err(self.throw_error(b"DivisionByZeroError", "Modulo by zero"));
                }
                // i64::MIN % -1 overflows in Rust; PHP defines it as 0.
                Num::Int(x.checked_rem(y).unwrap_or(0))
            }
            (ArithOp::Pow, Num::Int(x), Num::Int(y)) if y >= 0 => {
                int_pow(x, y).map_or(Num::Float((x as f64).powf(y as f64)), Num::Int)
            }
            (ArithOp::Pow, _, _) => Num::Float(a.float().powf(b.float())),
        };

        self.operand_stack.push(self.arena.alloc(result.into_val()));
        Ok(())
    }

    /// Execute Add operation: $result = $left + $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - add_function
    #[inline]
    pub(crate) fn exec_add(&mut self) -> Result<(), VmError> {
        self.binary_arithmetic(ArithOp::Add)
    }

    /// Execute Sub operation: $result = $left - $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - sub_function
    #[inline]
    pub(crate) fn exec_sub(&mut self) -> Result<(), VmError> {
        self.binary_arithmetic(ArithOp::Sub)
    }

    /// Execute Mul operation: $result = $left * $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - mul_function
    #[inline]
    pub(crate) fn exec_mul(&mut self) -> Result<(), VmError> {
        self.binary_arithmetic(ArithOp::Mul)
    }

    /// Execute Div operation: $result = $left / $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - div_function
    #[inline]
    pub(crate) fn exec_div(&mut self) -> Result<(), VmError> {
        self.binary_arithmetic(ArithOp::Div)
    }

    /// Execute Mod operation: $result = $left % $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - mod_function
    #[inline]
    pub(crate) fn exec_mod(&mut self) -> Result<(), VmError> {
        self.binary_arithmetic(ArithOp::Mod)
    }

    /// Execute Pow operation: $result = $left ** $right
    /// Reference: $PHP_SRC_PATH/Zend/zend_operators.c - pow_function
    #[inline]
    pub(crate) fn exec_pow(&mut self) -> Result<(), VmError> {
        self.binary_arithmetic(ArithOp::Pow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::value::Val;
    use crate::runtime::context::EngineContext;
    use std::sync::Arc;

    #[test]
    fn test_add_integers() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(10));
        let right = vm.arena.alloc(Val::Int(32));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_add().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Int(42)));
    }

    #[test]
    fn test_add_floats() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Float(10.5));
        let right = vm.arena.alloc(Val::Float(20.7));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_add().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        if let Val::Float(f) = result_val.value {
            assert!((f - 31.2).abs() < 0.0001);
        } else {
            panic!("Expected float");
        }
    }

    #[test]
    fn test_add_int_and_float() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(10));
        let right = vm.arena.alloc(Val::Float(5.5));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_add().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        // Int + Float = Float
        if let Val::Float(f) = result_val.value {
            assert!((f - 15.5).abs() < 0.0001);
        } else {
            panic!("Expected float, got {:?}", result_val.value);
        }
    }

    #[test]
    fn test_subtract_integers() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(50));
        let right = vm.arena.alloc(Val::Int(8));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_sub().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Int(42)));
    }

    #[test]
    fn test_multiply_integers() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(6));
        let right = vm.arena.alloc(Val::Int(7));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_mul().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Int(42)));
    }

    #[test]
    fn test_divide_integers_exact() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(84));
        let right = vm.arena.alloc(Val::Int(2));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_div().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        // An exact integer division stays an int (84 / 2 === 42 in PHP)
        assert!(matches!(result_val.value, Val::Int(42)));
    }

    #[test]
    fn test_divide_integers_float_result() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(10));
        let right = vm.arena.alloc(Val::Int(3));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_div().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        // 10 / 3 = 3.333... (non-exact division returns float)
        if let Val::Float(f) = result_val.value {
            assert!((f - 3.333333).abs() < 0.001);
        } else {
            panic!("Expected float for non-exact division");
        }
    }

    #[test]
    fn test_modulo() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(10));
        let right = vm.arena.alloc(Val::Int(3));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_mod().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Int(1)));
    }

    #[test]
    fn test_power() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(2));
        let right = vm.arena.alloc(Val::Int(10));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_pow().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        // int ** non-negative int is an int unless it overflows (2 ** 10 === 1024)
        assert!(matches!(result_val.value, Val::Int(1024)));
    }

    #[test]
    fn test_add_with_numeric_string() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(10));
        let right = vm.arena.alloc(Val::String(b"32".to_vec().into()));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_add().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        // "32" converts to 32
        assert!(matches!(result_val.value, Val::Int(42)));
    }

    #[test]
    fn test_add_with_bool() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(41));
        let right = vm.arena.alloc(Val::Bool(true));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_add().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        // true converts to 1
        assert!(matches!(result_val.value, Val::Int(42)));
    }

    #[test]
    fn test_add_with_null() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(42));
        let right = vm.arena.alloc(Val::Null);

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_add().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        // null converts to 0
        assert!(matches!(result_val.value, Val::Int(42)));
    }

    #[test]
    fn test_negative_result() {
        let engine = Arc::new(EngineContext::new());
        let mut vm = VM::new(engine);

        let left = vm.arena.alloc(Val::Int(10));
        let right = vm.arena.alloc(Val::Int(52));

        vm.operand_stack.push(left);
        vm.operand_stack.push(right);

        vm.exec_sub().unwrap();

        let result = vm.operand_stack.pop().unwrap();
        let result_val = vm.arena.get(result);
        assert!(matches!(result_val.value, Val::Int(-42)));
    }
}
