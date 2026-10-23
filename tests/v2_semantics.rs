//! Engine v2 values against real PHP 8.4: every pair of a matrix of values
//! compared with <=>, == and ===, and each value converted to bool, int,
//! float and string, must match tests/fixtures/v2_semantics.txt (written by
//! examples/gen_v2_semantics.php under PHP 8.4).

use php_rs::v2::array::{Key, PhpArray};
use php_rs::v2::string::PhpStr;
use php_rs::v2::value::Value;
use std::cmp::Ordering;
use std::rc::Rc;

fn arr(pairs: &[(Key, Value)]) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in pairs {
        a.insert(k.clone(), v.clone());
    }
    Value::Array(Rc::new(a))
}

/// The same values, in the same order, as the generator's $values.
fn values() -> Vec<Value> {
    let s = |x: &str| Value::Str(PhpStr::from(x));
    let i = Key::Int;
    let k = |x: &str| Key::Str(PhpStr::from(x));
    vec![
        Value::Null,
        Value::Bool(true),
        Value::Bool(false),
        Value::Int(0),
        Value::Int(1),
        Value::Int(-1),
        Value::Float(1.5),
        Value::Float(-0.0),
        Value::Float(f64::INFINITY),
        Value::Float(f64::NAN),
        Value::Int(i64::MAX),
        Value::Float(i64::MAX as f64),
        s(""),
        s("0"),
        s("1"),
        s("1.0"),
        s("abc"),
        s("1e3"),
        s(" 1"),
        s("1 "),
        s("01"),
        s("-1"),
        s("a"),
        s("A"),
        s("9223372036854775808"),
        s(" "),
        s("0.0"),
        arr(&[]),
        arr(&[(i(0), Value::Int(1))]),
        arr(&[(i(0), Value::Int(1)), (i(1), Value::Int(2))]),
        arr(&[(k("a"), Value::Int(1))]),
        arr(&[(i(1), Value::Int(2)), (i(0), Value::Int(1))]),
        arr(&[(i(0), Value::Int(1)), (i(1), Value::Int(2))]),
        arr(&[(k("a"), Value::Int(1)), (k("b"), Value::Int(2))]),
        arr(&[(k("b"), Value::Int(2)), (k("a"), Value::Int(1))]),
    ]
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[test]
fn value_is_sixteen_bytes() {
    assert_eq!(std::mem::size_of::<Value>(), 16);
}

#[test]
fn matches_php_8_4() {
    let values = values();
    let fixture = include_str!("fixtures/v2_semantics.txt");
    let mut failures = Vec::new();
    for line in fixture.lines() {
        let parts: Vec<&str> = line.split(' ').collect();
        match parts[0] {
            "conv" => {
                let v = &values[parts[1].parse::<usize>().unwrap()];
                let got_bool = v.to_bool().to_string();
                let got_int = v.to_int().to_string();
                let got_float = match v {
                    Value::Array(_) => "x".to_string(),
                    _ => hex(&v.to_float().to_be_bytes()),
                };
                let got_str = v.to_php_string().map_or("ARRAY".into(), |s| hex(s.as_bytes()));
                let want = (parts[2], parts[3], parts[4], parts.get(5).copied().unwrap_or(""));
                if (got_bool.as_str(), got_int.as_str(), got_float.as_str(), got_str.as_str()) != want {
                    failures.push(format!("{line} | got {got_bool} {got_int} {got_float} {got_str}"));
                }
            }
            "cmp" => {
                let a = &values[parts[1].parse::<usize>().unwrap()];
                let b = &values[parts[2].parse::<usize>().unwrap()];
                let cmp = match a.compare(b) {
                    Some(Ordering::Less) => -1,
                    Some(Ordering::Equal) => 0,
                    Some(Ordering::Greater) | None => 1,
                };
                let got = (cmp, a.loose_eq(b) as i32, a.identical(b) as i32);
                let want = (
                    parts[3].parse::<i32>().unwrap(),
                    parts[4].parse::<i32>().unwrap(),
                    parts[5].parse::<i32>().unwrap(),
                );
                if got != want {
                    failures.push(format!("{line} | got {} {} {}", got.0, got.1, got.2));
                }
            }
            _ => {}
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}
