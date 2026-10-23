mod common;

use common::run_code;
use php_rs::core::value::Val;

#[test]
fn test_var_dump_anonymous_class_with_parent() {
    let val = run_code(
        r#"<?php
class SomeClass {}
interface SomeInterface {}
trait SomeTrait {}

ob_start();
var_dump(new class(10) extends SomeClass implements SomeInterface {
    private $num;

    public function __construct($num)
    {
        $this->num = $num;
    }

    use SomeTrait;
});
$output = ob_get_clean();

// Check that output contains SomeClass@anonymous
if (strpos($output, 'SomeClass@anonymous') !== false) {
    return true;
}
return false;
"#,
    );

    match val {
        Val::Bool(true) => {
            // Success
        }
        v => panic!("Expected true (var_dump should contain SomeClass@anonymous), got {:?}", v),
    }
}
