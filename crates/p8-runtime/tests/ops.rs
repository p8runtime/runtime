//! p8 operators against real PHP 8.4: every binary operator on every
//! pair of a value matrix must give PHP's result, exception, warnings and
//! deprecations (tests/fixtures/ops.txt, written by
//! crates/p8-runtime/tests/gen_ops.php under PHP 8.4).

use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ops::{BinOp, Diag, OpError, binary};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

/// The generator's $values, in order.
fn values() -> Vec<Value> {
    let s = |x: &str| Value::Str(PhpStr::from(x));
    let list = |items: &[(i64, Value)]| {
        let mut a = PhpArray::new();
        for (k, v) in items {
            a.insert(Key::Int(*k), v.clone());
        }
        Value::Array(Rc::new(a))
    };
    vec![
        Value::Null,
        Value::Bool(true),
        Value::Bool(false),
        Value::Int(0),
        Value::Int(1),
        Value::Int(-1),
        Value::Int(7),
        Value::Int(-7),
        Value::Int(3),
        Value::Int(i64::MAX),
        Value::Int(i64::MIN),
        Value::Float(1.5),
        Value::Float(-2.5),
        Value::Float(0.0),
        Value::Float(7.5),
        Value::Float(f64::INFINITY),
        s(""),
        s("0"),
        s("5"),
        s("1.5"),
        s("abc"),
        s("5 apples"),
        s(" 3"),
        s("1e2"),
        s("ab"),
        s("AB"),
        list(&[]),
        list(&[(0, Value::Int(1))]),
        list(&[(2, s("x"))]),
        Value::Int(64),
        Value::Int(2),
    ]
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// json_encode of the arrays the matrix produces (lists and int-keyed maps of scalars).
fn json(a: &PhpArray) -> String {
    let scalar = |v: &Value| match v {
        Value::Int(i) => i.to_string(),
        Value::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s.as_bytes())),
        other => format!("{:?}", other),
    };
    if a.is_list() {
        format!("[{}]", a.values().map(scalar).collect::<Vec<_>>().join(","))
    } else {
        let parts: Vec<String> = a
            .iter()
            .map(|(k, v)| match k {
                Key::Int(i) => format!("\"{}\":{}", i, scalar(v)),
                Key::Str(s) => format!("\"{}\":{}", String::from_utf8_lossy(s.as_bytes()), scalar(v)),
            })
            .collect();
        format!("{{{}}}", parts.join(","))
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::Null => "N".into(),
        Value::Bool(b) => format!("b:{}", u8::from(*b)),
        Value::Int(i) => format!("i:{}", i),
        Value::Float(f) => format!("f:{}", hex(&f.to_be_bytes())),
        Value::Str(s) => format!("s:{}", hex(s.as_bytes())),
        Value::Array(a) => format!("a:{}", hex(json(a).as_bytes())),
        Value::Ref(_) | Value::Undef | Value::Object(_) => "ref".into(),
    }
}

fn op(symbol: &str) -> BinOp {
    match symbol {
        "+" => BinOp::Add,
        "-" => BinOp::Sub,
        "*" => BinOp::Mul,
        "/" => BinOp::Div,
        "%" => BinOp::Mod,
        "**" => BinOp::Pow,
        "." => BinOp::Concat,
        "&" => BinOp::BitAnd,
        "|" => BinOp::BitOr,
        "^" => BinOp::BitXor,
        "<<" => BinOp::Shl,
        ">>" => BinOp::Shr,
        other => panic!("unknown operator {other}"),
    }
}

#[test]
fn binary_operators_match_php_8_4() {
    let values = values();
    let mut failures = Vec::new();
    for line in include_str!("fixtures/ops.txt").lines() {
        let parts: Vec<&str> = line.split(' ').collect();
        let (a, b) = (&values[parts[1].parse::<usize>().unwrap()], &values[parts[2].parse::<usize>().unwrap()]);
        let mut diags = Vec::new();
        let result = match binary(op(parts[0]), a, b, &mut diags) {
            Ok(v) => show(&v),
            Err(e) => {
                let (class, msg) = match e {
                    OpError::Type(m) => ("TypeError", m),
                    OpError::DivisionByZero(m) => ("DivisionByZeroError", m.to_string()),
                    OpError::Arithmetic(m) => ("ArithmeticError", m.to_string()),
                    OpError::Error(m) => ("Error", m),
                };
                format!("E:{}:{}", class, hex(msg.as_bytes()))
            }
        };
        let diags: Vec<String> = diags
            .iter()
            .map(|d| match d {
                Diag::Warning(m) => hex(format!("W:{m}").as_bytes()),
                Diag::Deprecated(m) => hex(format!("D:{m}").as_bytes()),
            })
            .collect();
        let got = if diags.is_empty() { result } else { format!("{} {}", result, diags.join("|")) };
        let want = parts[3..].join(" ");
        if got != want {
            failures.push(format!("{} {} {} want {} got {}", parts[0], parts[1], parts[2], want, got));
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures[..failures.len().min(40)].join("\n"));
}
