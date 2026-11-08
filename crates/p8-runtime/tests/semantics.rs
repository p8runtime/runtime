//! p8 values against real PHP 8.6: every pair of a matrix of values
//! compared with <=>, == and ===, and each value converted to bool, int,
//! float and string, must match tests/fixtures/semantics.txt (written by
//! crates/p8-runtime/tests/gen_semantics.php under PHP 8.6; the values are
//! the same as 8.4's).

use p8_runtime::array::{Key, PhpArray};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cmp::Ordering;

fn arr(pairs: &[(Key, Value)]) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in pairs {
        a.insert(k.clone(), v.clone());
    }
    Value::array(a)
}

/// The same values, in the same order, as the generator's $values.
fn values() -> Vec<Value> {
    let s = |x: &str| Value::str(PhpStr::from(x));
    let i = Key::Int;
    let k = |x: &str| Key::str(PhpStr::from(x));
    vec![
        Value::NULL,
        Value::TRUE,
        Value::FALSE,
        Value::int(0),
        Value::int(1),
        Value::int(-1),
        Value::float(1.5),
        Value::float(-0.0),
        Value::float(f64::INFINITY),
        Value::float(f64::NAN),
        Value::int(i64::MAX),
        Value::float(i64::MAX as f64),
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
        arr(&[(i(0), Value::int(1))]),
        arr(&[(i(0), Value::int(1)), (i(1), Value::int(2))]),
        arr(&[(k("a"), Value::int(1))]),
        arr(&[(i(1), Value::int(2)), (i(0), Value::int(1))]),
        arr(&[(i(0), Value::int(1)), (i(1), Value::int(2))]),
        arr(&[(k("a"), Value::int(1)), (k("b"), Value::int(2))]),
        arr(&[(k("b"), Value::int(2)), (k("a"), Value::int(1))]),
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
fn matches_php_8_6() {
    let values = values();
    let fixture = include_str!("fixtures/semantics.txt");
    let mut failures = Vec::new();
    for line in fixture.lines() {
        let parts: Vec<&str> = line.split(' ').collect();
        match parts[0] {
            "conv" => {
                let v = &values[parts[1].parse::<usize>().unwrap()];
                let got_bool = v.to_bool().to_string();
                let got_int = v.to_int().to_string();
                let got_float = if v.is_array() { "x".to_string() } else { hex(&v.to_float().to_be_bytes()) };
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
