mod common;
use common::*;
use php_rs::core::value::Val;

#[test]
fn test_serialize_null() {
    let result = run_php(r#"<?php return serialize(null);"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"N;".to_vec())));
}

#[test]
fn test_serialize_bool_true() {
    let result = run_php(r#"<?php return serialize(true);"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"b:1;".to_vec())));
}

#[test]
fn test_serialize_bool_false() {
    let result = run_php(r#"<?php return serialize(false);"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"b:0;".to_vec())));
}

#[test]
fn test_serialize_int_positive() {
    let result = run_php(r#"<?php return serialize(42);"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"i:42;".to_vec())));
}

#[test]
fn test_serialize_int_negative() {
    let result = run_php(r#"<?php return serialize(-123);"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"i:-123;".to_vec())));
}

#[test]
fn test_serialize_int_zero() {
    let result = run_php(r#"<?php return serialize(0);"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"i:0;".to_vec())));
}

#[test]
fn test_serialize_float() {
    let result = run_php(r#"<?php return serialize(3.14);"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"d:3.14;".to_vec())));
}

#[test]
fn test_serialize_float_negative() {
    let result = run_php(r#"<?php return serialize(-2.5);"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"d:-2.5;".to_vec())));
}

#[test]
fn test_serialize_string() {
    let result = run_php(r#"<?php return serialize("hello");"#);
    assert_eq!(
        result,
        Val::String(std::rc::Rc::new(b"s:5:\"hello\";".to_vec()))
    );
}

#[test]
fn test_serialize_string_empty() {
    let result = run_php(r#"<?php return serialize("");"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"s:0:\"\";".to_vec())));
}

#[test]
fn test_serialize_string_with_quotes() {
    let result = run_php(r#"<?php return serialize('He said "hi"');"#);
    assert_eq!(
        result,
        Val::String(std::rc::Rc::new(b"s:12:\"He said \"hi\"\";".to_vec()))
    );
}

#[test]
fn test_serialize_empty_array() {
    let result = run_php(r#"<?php return serialize([]);"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"a:0:{}".to_vec())));
}

#[test]
fn test_serialize_indexed_array() {
    let result = run_php(r#"<?php return serialize([1, 2, 3]);"#);
    assert_eq!(
        result,
        Val::String(std::rc::Rc::new(b"a:3:{i:0;i:1;i:1;i:2;i:2;i:3;}".to_vec()))
    );
}

#[test]
fn test_serialize_associative_array() {
    let result = run_php(r#"<?php return serialize(["a" => 1, "b" => 2]);"#);
    // Note: HashMap ordering may vary, so we check if it contains the right parts
    match result {
        Val::String(s) => {
            let serialized = String::from_utf8_lossy(&s);
            assert!(serialized.starts_with("a:2:{"));
            assert!(serialized.contains("s:1:\"a\";i:1;"));
            assert!(serialized.contains("s:1:\"b\";i:2;"));
            assert!(serialized.ends_with("}"));
        }
        _ => panic!("Expected string result"),
    }
}

#[test]
fn test_serialize_mixed_array() {
    let result = run_php(r#"<?php return serialize([0 => "zero", "key" => "value", 5 => "five"]);"#);
    match result {
        Val::String(s) => {
            let serialized = String::from_utf8_lossy(&s);
            assert!(serialized.starts_with("a:3:{"));
            assert!(serialized.contains("i:0;s:4:\"zero\";"));
            assert!(serialized.contains("s:3:\"key\";s:5:\"value\";"));
            assert!(serialized.contains("i:5;s:4:\"five\";"));
            assert!(serialized.ends_with("}"));
        }
        _ => panic!("Expected string result"),
    }
}

#[test]
fn test_serialize_nested_array() {
    let result = run_php(r#"<?php return serialize([1, [2, 3], 4]);"#);
    assert_eq!(
        result,
        Val::String(std::rc::Rc::new(
            b"a:3:{i:0;i:1;i:1;a:2:{i:0;i:2;i:1;i:3;}i:2;i:4;}".to_vec()
        ))
    );
}

#[test]
fn test_serialize_object() {
    let result = run_php(
        r#"<?php
        class SimpleClass {
            public $name = "test";
            public $value = 42;
        }
        $obj = new SimpleClass();
        return serialize($obj);
    "#,
    );
    
    match result {
        Val::String(s) => {
            let serialized = String::from_utf8_lossy(&s);
            assert!(serialized.starts_with("O:11:\"SimpleClass\":2:{"));
            assert!(serialized.contains("s:4:\"name\";s:4:\"test\";"));
            assert!(serialized.contains("s:5:\"value\";i:42;"));
            assert!(serialized.ends_with("}"));
        }
        _ => panic!("Expected string result"),
    }
}

#[test]
fn test_unserialize_null() {
    let result = run_php(r#"<?php var_dump(unserialize("N;"));"#);
    assert_eq!(result, Val::Null);
}

#[test]
fn test_unserialize_bool_true() {
    let result = run_php(r#"<?php $x = unserialize("b:1;"); return $x ? "true" : "false";"#);
    assert_eq!(
        result,
        Val::String(std::rc::Rc::new(b"true".to_vec()))
    );
}

#[test]
fn test_unserialize_bool_false() {
    let result = run_php(r#"<?php $x = unserialize("b:0;"); return $x ? "true" : "false";"#);
    assert_eq!(
        result,
        Val::String(std::rc::Rc::new(b"false".to_vec()))
    );
}

#[test]
fn test_unserialize_int() {
    let result = run_php(r#"<?php return unserialize("i:42;");"#);
    assert_eq!(result, Val::Int(42));
}

#[test]
fn test_unserialize_negative_int() {
    let result = run_php(r#"<?php return unserialize("i:-123;");"#);
    assert_eq!(result, Val::Int(-123));
}

#[test]
fn test_unserialize_float() {
    let result = run_php(r#"<?php return unserialize("d:3.14;");"#);
    assert_eq!(result, Val::Float(3.14));
}

#[test]
fn test_unserialize_string() {
    let result = run_php(r#"<?php return unserialize('s:5:"hello";');"#);
    assert_eq!(
        result,
        Val::String(std::rc::Rc::new(b"hello".to_vec()))
    );
}

#[test]
fn test_unserialize_empty_string() {
    let result = run_php(r#"<?php return unserialize('s:0:"";');"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"".to_vec())));
}

#[test]
fn test_unserialize_empty_array() {
    let result = run_php(r#"<?php return unserialize("a:0:{}");"#);
    match result {
        Val::Array(arr) => {
            assert_eq!(arr.map.len(), 0);
        }
        _ => panic!("Expected array result"),
    }
}

#[test]
fn test_unserialize_indexed_array() {
    let result = run_php(r#"<?php $arr = unserialize("a:3:{i:0;i:1;i:1;i:2;i:2;i:3;}"); return $arr[0] . "," . $arr[1] . "," . $arr[2];"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"1,2,3".to_vec())));
}

#[test]
fn test_unserialize_associative_array() {
    let result = run_php(
        r#"<?php 
        $arr = unserialize('a:2:{s:1:"a";i:1;s:1:"b";i:2;}');
        return $arr["a"] . "," . $arr["b"];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"1,2".to_vec())));
}

#[test]
fn test_unserialize_nested_array() {
    let result = run_php(
        r#"<?php 
        $arr = unserialize("a:3:{i:0;i:1;i:1;a:2:{i:0;i:2;i:1;i:3;}i:2;i:4;}");
        return $arr[0] . "," . $arr[1][0] . "," . $arr[1][1] . "," . $arr[2];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"1,2,3,4".to_vec())));
}

#[test]
fn test_serialize_unserialize_roundtrip_null() {
    let result = run_php(
        r#"<?php 
        $original = null;
        $serialized = serialize($original);
        $result = unserialize($serialized);
        var_dump($result);
    "#,
    );
    assert_eq!(result, Val::Null);
}

#[test]
fn test_serialize_unserialize_roundtrip_bool() {
    let result = run_php(
        r#"<?php 
        $serialized = serialize(true);
        $result = unserialize($serialized);
        return $result ? "true" : "false";
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"true".to_vec())));
}

#[test]
fn test_serialize_unserialize_roundtrip_int() {
    let result = run_php(
        r#"<?php 
        $serialized = serialize(12345);
        return unserialize($serialized);
    "#,
    );
    assert_eq!(result, Val::Int(12345));
}

#[test]
fn test_serialize_unserialize_roundtrip_float() {
    let result = run_php(
        r#"<?php 
        $serialized = serialize(3.14159);
        return unserialize($serialized);
    "#,
    );
    assert_eq!(result, Val::Float(3.14159));
}

#[test]
fn test_serialize_unserialize_roundtrip_string() {
    let result = run_php(
        r#"<?php 
        $serialized = serialize("hello world");
        return unserialize($serialized);
    "#,
    );
    assert_eq!(
        result,
        Val::String(std::rc::Rc::new(b"hello world".to_vec()))
    );
}

#[test]
fn test_serialize_unserialize_roundtrip_array() {
    let result = run_php(
        r#"<?php 
        $original = [1, 2, 3, 4, 5];
        $serialized = serialize($original);
        $result = unserialize($serialized);
        return $result[0] + $result[1] + $result[2] + $result[3] + $result[4];
    "#,
    );
    assert_eq!(result, Val::Int(15));
}

#[test]
fn test_serialize_unserialize_roundtrip_assoc_array() {
    let result = run_php(
        r#"<?php 
        $original = ["name" => "John", "age" => 30, "city" => "NYC"];
        $serialized = serialize($original);
        $result = unserialize($serialized);
        return $result["name"] . "," . $result["age"] . "," . $result["city"];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"John,30,NYC".to_vec())));
}

#[test]
fn test_serialize_unserialize_roundtrip_nested_array() {
    let result = run_php(
        r#"<?php 
        $original = ["a" => [1, 2], "b" => [3, 4]];
        $serialized = serialize($original);
        $result = unserialize($serialized);
        return $result["a"][0] . "," . $result["a"][1] . "," . $result["b"][0] . "," . $result["b"][1];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"1,2,3,4".to_vec())));
}

#[test]
fn test_unserialize_object() {
    let result = run_php(
        r#"<?php
        class SimpleClass {
            public $name;
            public $value;
        }
        $obj = unserialize('O:11:"SimpleClass":2:{s:4:"name";s:4:"test";s:5:"value";i:42;}');
        return $obj->name . "," . $obj->value;
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"test,42".to_vec())));
}

#[test]
fn test_serialize_unserialize_roundtrip_object() {
    let result = run_php(
        r#"<?php
        class Person {
            public $name;
            public $age;
        }
        $original = new Person();
        $original->name = "Alice";
        $original->age = 25;
        
        $serialized = serialize($original);
        $result = unserialize($serialized);
        return $result->name . "," . $result->age;
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"Alice,25".to_vec())));
}

#[test]
fn test_unserialize_invalid_data() {
    let result = run_php(r#"<?php return unserialize("invalid");"#);
    // unserialize returns false on error
    assert_eq!(result, Val::Bool(false));
}

#[test]
fn test_unserialize_truncated_data() {
    let result = run_php(r#"<?php return unserialize("s:10:");"#);
    // unserialize returns false on error
    assert_eq!(result, Val::Bool(false));
}

#[test]
fn test_serialize_complex_nested_structure() {
    let result = run_php(
        r#"<?php 
        $data = [
            "users" => [
                ["name" => "Alice", "score" => 100],
                ["name" => "Bob", "score" => 85]
            ],
            "total" => 2
        ];
        $serialized = serialize($data);
        $restored = unserialize($serialized);
        return $restored["users"][0]["name"] . "," . $restored["users"][1]["score"] . "," . $restored["total"];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"Alice,85,2".to_vec())));
}

#[test]
fn test_serialize_preserves_numeric_string_keys() {
    let result = run_php(
        r#"<?php 
        $arr = ["0" => "a", "1" => "b", "10" => "c"];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        return $restored["0"] . $restored["1"] . $restored["10"];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"abc".to_vec())));
}

#[test]
fn test_serialize_array_with_gaps() {
    let result = run_php(
        r#"<?php 
        $arr = [];
        $arr[0] = "a";
        $arr[5] = "b";
        $arr[10] = "c";
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        return $restored[0] . $restored[5] . $restored[10];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"abc".to_vec())));
}

#[test]
fn test_serialize_missing_argument() {
    let code = r#"<?php serialize();"#;
    let result = std::panic::catch_unwind(|| run_php(code));
    assert!(result.is_err(), "Expected error when calling serialize() without arguments");
}

#[test]
fn test_unserialize_missing_argument() {
    let code = r#"<?php unserialize();"#;
    let result = std::panic::catch_unwind(|| run_php(code));
    assert!(result.is_err(), "Expected error when calling unserialize() without arguments");
}

#[test]
fn test_unserialize_non_string() {
    let code = r#"<?php unserialize(123);"#;
    let result = std::panic::catch_unwind(|| run_php(code));
    assert!(result.is_err(), "Expected error when calling unserialize() with non-string");
}

// ========== Additional Edge Case Tests ==========

#[test]
fn test_serialize_deeply_nested_array() {
    let result = run_php(
        r#"<?php 
        $data = ["a" => ["b" => ["c" => ["d" => "deep"]]]];
        return serialize($data);
    "#,
    );
    match result {
        Val::String(s) => {
            let serialized = String::from_utf8_lossy(&s);
            assert!(serialized.contains("deep"));
            // Verify it can be unserialized back
            let code = format!(r#"<?php return unserialize('{}')["a"]["b"]["c"]["d"];"#, serialized);
            let restored = run_php(&code);
            assert_eq!(restored, Val::String(std::rc::Rc::new(b"deep".to_vec())));
        }
        _ => panic!("Expected string result"),
    }
}

#[test]
fn test_serialize_array_with_mixed_keys() {
    let result = run_php(
        r#"<?php 
        $arr = [0 => "zero", "1" => "one", 2 => "two", "key" => "value"];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        return $restored[0] . "," . $restored["1"] . "," . $restored[2] . "," . $restored["key"];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"zero,one,two,value".to_vec())));
}

#[test]
fn test_serialize_large_numbers() {
    let result = run_php(
        r#"<?php 
        $big = 9223372036854775807; // PHP_INT_MAX
        $serialized = serialize($big);
        $restored = unserialize($serialized);
        return $restored;
    "#,
    );
    assert_eq!(result, Val::Int(9223372036854775807));
}

#[test]
fn test_serialize_negative_float() {
    let result = run_php(r#"<?php return serialize(-123.456);"#);
    match result {
        Val::String(s) => {
            let serialized = String::from_utf8_lossy(&s);
            assert!(serialized.starts_with("d:-123.456"));
        }
        _ => panic!("Expected string result"),
    }
}

#[test]
fn test_serialize_float_zero() {
    let result = run_php(r#"<?php return serialize(0.0);"#);
    assert_eq!(result, Val::String(std::rc::Rc::new(b"d:0;".to_vec())));
}

#[test]
fn test_serialize_string_with_newlines() {
    let result = run_php(
        r#"<?php 
        $str = "line1\nline2\nline3";
        $serialized = serialize($str);
        $restored = unserialize($serialized);
        return $restored;
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"line1\nline2\nline3".to_vec())));
}

#[test]
fn test_serialize_string_with_special_chars() {
    let result = run_php(
        r#"<?php 
        $str = "tab\there";
        $serialized = serialize($str);
        $restored = unserialize($serialized);
        return $restored;
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"tab\there".to_vec())));
}

#[test]
fn test_serialize_empty_string_in_array() {
    let result = run_php(
        r#"<?php 
        $arr = ["", "test", ""];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        return $restored[0] . "|" . $restored[1] . "|" . $restored[2];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"|test|".to_vec())));
}

#[test]
fn test_serialize_array_with_null_values() {
    let result = run_php(
        r#"<?php 
        $arr = [1, null, 2, null, 3];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        // Check that null values are preserved
        return ($restored[1] === null && $restored[3] === null) ? "ok" : "fail";
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"ok".to_vec())));
}

#[test]
fn test_serialize_array_with_bool_values() {
    let result = run_php(
        r#"<?php 
        $arr = [true, false, true];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        return ($restored[0] ? "T" : "F") . ($restored[1] ? "T" : "F") . ($restored[2] ? "T" : "F");
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"TFT".to_vec())));
}

#[test]
fn test_serialize_multidimensional_array() {
    let result = run_php(
        r#"<?php 
        $matrix = [
            [1, 2, 3],
            [4, 5, 6],
            [7, 8, 9]
        ];
        $serialized = serialize($matrix);
        $restored = unserialize($serialized);
        return $restored[0][0] + $restored[1][1] + $restored[2][2];
    "#,
    );
    assert_eq!(result, Val::Int(15)); // 1 + 5 + 9
}

#[test]
fn test_serialize_object_with_numeric_properties() {
    let result = run_php(
        r#"<?php
        class NumericProps {
            public $zero;
            public $one;
            public $negative;
        }
        $obj = new NumericProps();
        $obj->zero = 0;
        $obj->one = 1;
        $obj->negative = -5;
        $serialized = serialize($obj);
        $restored = unserialize($serialized);
        return $restored->zero + $restored->one + $restored->negative;
    "#,
    );
    assert_eq!(result, Val::Int(-4));
}

#[test]
fn test_serialize_object_with_string_properties() {
    let result = run_php(
        r#"<?php
        class StringProps {
            public $first = "Hello";
            public $second = "World";
        }
        $obj = new StringProps();
        $serialized = serialize($obj);
        $restored = unserialize($serialized);
        return $restored->first . " " . $restored->second;
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"Hello World".to_vec())));
}

#[test]
fn test_serialize_object_with_array_property() {
    let result = run_php(
        r#"<?php
        class WithArray {
            public $items;
        }
        $obj = new WithArray();
        $obj->items = [1, 2, 3];
        $serialized = serialize($obj);
        $restored = unserialize($serialized);
        return $restored->items[0] + $restored->items[1] + $restored->items[2];
    "#,
    );
    assert_eq!(result, Val::Int(6));
}

#[test]
fn test_serialize_object_with_null_property() {
    let result = run_php(
        r#"<?php
        class WithNull {
            public $value;
        }
        $obj = new WithNull();
        $obj->value = null;
        $serialized = serialize($obj);
        $restored = unserialize($serialized);
        return $restored->value === null ? "null" : "not null";
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"null".to_vec())));
}

#[test]
fn test_serialize_object_with_bool_property() {
    let result = run_php(
        r#"<?php
        class WithBool {
            public $flag = true;
        }
        $obj = new WithBool();
        $serialized = serialize($obj);
        $restored = unserialize($serialized);
        return $restored->flag ? "yes" : "no";
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"yes".to_vec())));
}

#[test]
fn test_serialize_array_keys_preserved() {
    let result = run_php(
        r#"<?php 
        $arr = [10 => "ten", 20 => "twenty", 30 => "thirty"];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        return $restored[10] . "," . $restored[20] . "," . $restored[30];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"ten,twenty,thirty".to_vec())));
}

#[test]
fn test_unserialize_maintains_array_order() {
    let result = run_php(
        r#"<?php 
        $arr = ["z" => 1, "a" => 2, "m" => 3];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        $keys = array_keys($restored);
        return $keys[0] . $keys[1] . $keys[2];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"zam".to_vec())));
}

#[test]
fn test_serialize_very_long_string() {
    let result = run_php(
        r#"<?php 
        $long = str_repeat("x", 1000);
        $serialized = serialize($long);
        $restored = unserialize($serialized);
        return strlen($restored);
    "#,
    );
    assert_eq!(result, Val::Int(1000));
}

#[test]
fn test_serialize_array_with_float_values() {
    let result = run_php(
        r#"<?php 
        $arr = [1.1, 2.2, 3.3];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        return $restored[0] + $restored[1] + $restored[2];
    "#,
    );
    match result {
        Val::Float(f) => {
            assert!((f - 6.6).abs() < 0.01);
        }
        _ => panic!("Expected float result"),
    }
}

#[test]
fn test_serialize_single_element_array() {
    let result = run_php(
        r#"<?php 
        $arr = ["only"];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        return $restored[0];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"only".to_vec())));
}

#[test]
fn test_serialize_associative_with_integer_keys() {
    let result = run_php(
        r#"<?php 
        $arr = [100 => "hundred", 200 => "two hundred"];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        return $restored[100] . " and " . $restored[200];
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"hundred and two hundred".to_vec())));
}

#[test]
fn test_serialize_object_modified_properties() {
    let result = run_php(
        r#"<?php
        class Mutable {
            public $value = 10;
        }
        $obj = new Mutable();
        $obj->value = 20;
        $obj->extra = 30;
        $serialized = serialize($obj);
        $restored = unserialize($serialized);
        return $restored->value + $restored->extra;
    "#,
    );
    assert_eq!(result, Val::Int(50));
}

#[test]
fn test_unserialize_format_validation() {
    // Test that malformed serialized data returns false
    let result = run_php(r#"<?php return unserialize("i:not_a_number;");"#);
    assert_eq!(result, Val::Bool(false));
}

#[test]
fn test_serialize_array_nested_empty() {
    let result = run_php(
        r#"<?php 
        $arr = [[], [1], [[]]];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        return count($restored[0]) + count($restored[1]) + count($restored[2]);
    "#,
    );
    assert_eq!(result, Val::Int(2)); // 0 + 1 + 1
}

#[test]
fn test_serialize_zero_and_empty_string_distinct() {
    let result = run_php(
        r#"<?php 
        $arr = [0, "", false, null];
        $serialized = serialize($arr);
        $restored = unserialize($serialized);
        // Verify distinct types by checking values and comparisons
        $checks = [];
        $checks[] = ($restored[0] === 0) ? "int" : "not_int";
        $checks[] = ($restored[1] === "") ? "str" : "not_str";
        $checks[] = ($restored[2] === false) ? "bool" : "not_bool";
        $checks[] = ($restored[3] === null) ? "null" : "not_null";
        return implode(",", $checks);
    "#,
    );
    assert_eq!(result, Val::String(std::rc::Rc::new(b"int,str,bool,null".to_vec())));
}
