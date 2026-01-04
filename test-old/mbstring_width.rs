mod common;

use common::run_code;
use php_rs::core::value::Val;

#[test]
fn mb_strwidth_ascii() {
    let val = run_code("<?php return mb_strwidth('abc');");
    assert_eq!(val, Val::Int(3));
}

#[test]
fn mb_strimwidth_basic() {
    let val = run_code("<?php return mb_strimwidth('abcdef', 0, 3, '..');");
    assert_eq!(val, Val::String(b"ab..".to_vec().into()));
}
