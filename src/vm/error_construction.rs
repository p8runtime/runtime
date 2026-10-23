//! Error construction helpers
//!
//! Provides convenient methods for creating specific VmError variants,
//! making error handling more ergonomic throughout the codebase.

use crate::vm::engine::VmError;

impl VmError {
    /// Create a stack underflow error for a specific operation
    pub fn stack_underflow(operation: &'static str) -> Self {
        VmError::StackUnderflow { operation }
    }

    /// Create a type error
    pub fn type_error(
        expected: impl Into<String>,
        got: impl Into<String>,
        operation: &'static str,
    ) -> Self {
        VmError::TypeError {
            expected: expected.into(),
            got: got.into(),
            operation,
        }
    }

    /// Create an undefined variable error
    pub fn undefined_variable(name: impl Into<String>) -> Self {
        VmError::UndefinedVariable { name: name.into() }
    }

    /// Create an undefined function error
    pub fn undefined_function(name: impl Into<String>) -> Self {
        VmError::UndefinedFunction { name: name.into() }
    }

    /// Create an undefined method error
    pub fn undefined_method(class: impl Into<String>, method: impl Into<String>) -> Self {
        VmError::UndefinedMethod {
            class: class.into(),
            method: method.into(),
        }
    }

    /// Create a division by zero error
    pub fn division_by_zero() -> Self {
        VmError::DivisionByZero
    }

    /// Create a generic runtime error (for backward compatibility)
    pub fn runtime(message: impl Into<String>) -> Self {
        VmError::RuntimeError(message.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_construction() {
        let err = VmError::stack_underflow("test_op");
        assert!(matches!(
            err,
            VmError::StackUnderflow {
                operation: "test_op"
            }
        ));

        let err = VmError::type_error("int", "string", "add");
        match err {
            VmError::TypeError {
                expected,
                got,
                operation,
            } => {
                assert_eq!(expected, "int");
                assert_eq!(got, "string");
                assert_eq!(operation, "add");
            }
            _ => panic!("Wrong error variant"),
        }

        let err = VmError::undefined_variable("foo");
        match err {
            VmError::UndefinedVariable { name } => {
                assert_eq!(name, "foo");
            }
            _ => panic!("Wrong error variant"),
        }
    }

    #[test]
    fn test_error_display() {
        let err = VmError::stack_underflow("pop");
        assert_eq!(err.to_string(), "Stack underflow during pop");

        let err = VmError::type_error("int", "string", "add");
        assert_eq!(
            err.to_string(),
            "Type error in add: expected int, got string"
        );

        let err = VmError::undefined_variable("count");
        assert_eq!(err.to_string(), "Undefined variable: $count");

        let err = VmError::division_by_zero();
        assert_eq!(err.to_string(), "Division by zero");
    }
}

/// The Throwable PHP raises for an engine error message, for the opcode
/// handlers that still report a PHP runtime error as `RuntimeError`: the run
/// loop throws it as that class so scripts can catch it. None for internal
/// failures and for fatal errors PHP does not throw.
/// Reference: $PHP_SRC_PATH/Zend/zend_execute.c, zend_object_handlers.c
pub(crate) fn thrown_class_for(message: &str) -> Option<&'static [u8]> {
    const ERRORS: &[&str] = &[
        "Cannot access private property",
        "Cannot access protected property",
        "Cannot access self::",
        "Cannot access parent::",
        "Cannot access static::",
        "Call to undefined method",
        "Call to undefined function",
        "Call to private method",
        "Call to protected method",
        "Call to private ",
        "Call to protected ",
        "Call to a member function",
        "Cannot instantiate ",
        "Using $this when not in object context",
        "Cannot modify readonly property",
        "Cannot use object of type",
        "Cannot use a scalar value as an array",
        "Undefined constant",
        "Can only throw objects",
        "Class \"",
        "Access to undeclared static property",
        "Non-static method",
        "Attempt to assign property",
        "Attempt to increment/decrement property",
        "Cannot resume an already running generator",
        "Object of class",
        "Value of type",
        "Cannot call abstract method",
        "Cannot unset string offsets",
        "[] operator not supported for strings",
        "__clone method called on non-object",
    ];
    if message.starts_with("Modulo by zero") {
        return Some(b"DivisionByZeroError");
    }
    if message.starts_with("Cannot assign ") && message.contains(" to property ") {
        return Some(b"TypeError");
    }
    ERRORS
        .iter()
        .any(|p| message.starts_with(p))
        .then_some(b"Error".as_slice())
}
