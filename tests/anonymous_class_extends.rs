mod common;

use common::run_code;
use php_rs::core::value::Val;

#[test]
fn test_anonymous_class_simple() {
    let val = run_code(
        r#"<?php
$obj = new class() {
    public function test() {
        return "works";
    }
};
return $obj->test();
"#,
    );

    match val {
        Val::String(s) => {
            assert_eq!(String::from_utf8_lossy(&s), "works");
        }
        v => panic!("Expected string 'works', got {:?}", v),
    }
}

#[test]
fn test_anonymous_class_extends() {
    let val = run_code(
        r#"<?php
class SomeClass {}

$obj = new class(10) extends SomeClass {
    private $num;

    public function __construct($num)
    {
        $this->num = $num;
    }
};

return get_class($obj);
"#,
    );

    match val {
        Val::String(s) => {
            let class_name = String::from_utf8_lossy(&s);
            // Should contain "SomeClass@anonymous" (parent class name)
            assert!(
                class_name.contains("SomeClass@anonymous"),
                "Expected 'SomeClass@anonymous', got: {}",
                class_name
            );
        }
        v => panic!("Expected string with SomeClass@anonymous, got {:?}", v),
    }
}

#[test]
fn test_anonymous_class_extends_and_implements() {
    let val = run_code(
        r#"<?php
class SomeClass {}
interface SomeInterface {}

$obj = new class(10) extends SomeClass implements SomeInterface {
    private $num;

    public function __construct($num)
    {
        $this->num = $num;
    }
};

return $obj;
"#,
    );

    match val {
        Val::Object(_) => {
            // Success - we created an object
        }
        v => panic!("Expected object, got {:?}", v),
    }
}

#[test]
fn test_anonymous_class_with_trait() {
    let val = run_code(
        r#"<?php
trait SomeTrait {
    public function traitMethod() {
        return "from trait";
    }
}

$obj = new class() {
    use SomeTrait;
};

return $obj->traitMethod();
"#,
    );

    match val {
        Val::String(s) => {
            assert_eq!(String::from_utf8_lossy(&s), "from trait");
        }
        v => panic!("Expected string 'from trait', got {:?}", v),
    }
}

#[test]
fn test_anonymous_class_constructor_args() {
    let val = run_code(
        r#"<?php
$value = 42;
$obj = new class($value) {
    private $val;
    
    public function __construct($v) {
        $this->val = $v;
    }
    
    public function getValue() {
        return $this->val;
    }
};

return $obj->getValue();
"#,
    );

    match val {
        Val::Int(n) => {
            assert_eq!(n, 42);
        }
        v => panic!("Expected int 42, got {:?}", v),
    }
}

#[test]
fn test_anonymous_class_instanceof() {
    let val = run_code(
        r#"<?php
class BaseClass {}
interface MyInterface {}

$obj = new class extends BaseClass implements MyInterface {};

// Test instanceof for parent class
$isBaseClass = $obj instanceof BaseClass;
// Test instanceof for interface
$isMyInterface = $obj instanceof MyInterface;

return $isBaseClass && $isMyInterface;
"#,
    );

    match val {
        Val::Bool(true) => {
            // Success - object is instance of both BaseClass and MyInterface
        }
        v => panic!("Expected true, got {:?}", v),
    }
}
