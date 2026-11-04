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

/// The fixture's expected result for this machine. The NaN an invalid
/// operation makes (`INF - INF`, `0 * INF`, `(-1) ** 1.5`) is the
/// processor's "default NaN": negative on x86-64 (fff8…, where the fixtures
/// were recorded), positive on arm64 (7ff8…). PHP computes with C doubles,
/// so it gets this machine's bits too; no text it prints shows a NaN's sign
/// (zend_gcvt, php_conv_fp and sprintf print "NAN"/"NaN" alone), only
/// `pack()` does. See #239.
fn on_this_machine(want: &str) -> String {
    let nan = f64::INFINITY - std::hint::black_box(f64::INFINITY);
    want.replace("f:fff8000000000000", &format!("f:{}", hex(&nan.to_be_bytes())))
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
        let want = on_this_machine(&parts[3..].join(" "));
        if got != want {
            failures.push(format!("{} {} {} want {} got {}", parts[0], parts[1], parts[2], want, got));
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures[..failures.len().min(40)].join("\n"));
}

/// `-$x` is PHP's `$x * -1`: for a NaN, what this machine's multiply gives
/// (the NaN passed through with its sign, quieted), not a sign flip.
#[test]
fn negating_a_nan_is_multiplying_it_by_minus_one() {
    let minus_one = std::hint::black_box(-1.0f64);
    for bits in [0x7ff8_0000_0000_0000u64, 0xfff8_0000_0000_0000, 0x7ff8_0000_0000_0001, 0xfff0_0000_0000_0001] {
        let nan = f64::from_bits(bits);
        let negated = p8_runtime::ops::negate(&Value::Float(nan), &mut Vec::new()).expect("a float");
        let Value::Float(f) = negated else { panic!("a float") };
        assert_eq!(f.to_bits(), (nan * minus_one).to_bits(), "{bits:016x}");
    }
    let Ok(Value::Float(z)) = p8_runtime::ops::negate(&Value::Float(0.0), &mut Vec::new()) else { panic!("a float") };
    assert!(z == 0.0 && z.is_sign_negative());
}
