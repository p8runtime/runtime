//! `TypeDecl::plain_tags`, the set of value tags a declared type takes
//! with no test past the tag: for every type made of the kind bits (and
//! `mixed`), a value whose tag is in the set is one `quick` takes, and a
//! value of a kind `quick` takes whole (null, int, float, string, array)
//! has its tag in the set, so the one-bit test answers for it.

use p8_runtime::array::PhpArray;
use p8_runtime::string::IStr;
use p8_runtime::types::{T_ARRAY, T_BOOL, T_CALLABLE, T_FALSE, T_FLOAT, T_INT, T_MIXED, T_NULL, T_OBJECT, T_STATIC, T_STRING, T_TRUE, T_VOID, TypeDecl};
use p8_runtime::value::{Kind, Value};

#[test]
fn a_tag_in_the_plain_set_is_a_value_quick_takes() {
    let bits = [T_MIXED, T_STATIC, T_CALLABLE, T_OBJECT, T_ARRAY, T_STRING, T_INT, T_FLOAT, T_BOOL, T_FALSE, T_TRUE, T_VOID, T_NULL];
    let values = [
        Value::NULL,
        Value::UNDEF,
        Value::TRUE,
        Value::FALSE,
        Value::int(3),
        Value::float(0.5),
        Value::istr(IStr::intern(b"plain tags").expect("room")),
        Value::str("heap"),
        Value::array(PhpArray::new()),
        Value::new_reference(Value::int(1)),
    ];
    for set in 0u32..1 << bits.len() {
        let mask = bits.iter().enumerate().filter(|(i, _)| set & 1 << i != 0).fold(0, |m, (_, b)| m | b);
        let ty = TypeDecl::new(mask, Vec::new(), Vec::new(), false);
        let tags = ty.plain_tags();
        for v in &values {
            let tagged = tags & v.tag_bit() != 0;
            assert!(!tagged || ty.quick(v), "mask {mask:#x}: {:?} tagged but not quick", v.kind());
            if matches!(v.kind(), Kind::Null | Kind::Int | Kind::Float | Kind::Str | Kind::Array) {
                assert_eq!(tagged, ty.quick(v), "mask {mask:#x}: {:?}", v.kind());
            }
            if v.kind() == Kind::Bool && mask & (T_BOOL | T_MIXED) != 0 {
                assert!(tagged, "mask {mask:#x}: a bool");
            }
        }
        // An object's tag: only where every object is taken.
        assert_eq!(tags & Value::tags_of(Kind::Object) != 0, mask & (T_OBJECT | T_MIXED) != 0, "mask {mask:#x}");
        // Never an undefined value or a reference (the caller decides those).
        assert_eq!(tags & (Value::tags_of(Kind::Undef) | Value::tags_of(Kind::Ref)), 0);
    }
}
