//! Allocation and release (#412): where PHP shares one value (its empty
//! array, its known strings), p8 makes and frees nothing per use. Each
//! test runs a loop at two sizes: the counted work must not grow with the
//! iterations, and the output must be PHP's. Counted per thread by
//! p8_runtime::counts, which this crate's tests turn on (see hash_counts.rs).

use p8_runtime::counts::{Count, get};

#[derive(Default)]
struct Capture(Vec<u8>);

impl p8::Host for Capture {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

/// `body` with `$n` set to `n`, as one request: its output, and how many
/// of `c` it did.
fn run(body: &str, n: u32, c: Count) -> (String, u64) {
    let platform = p8::Platform::new(&[p8::extensions::STANDARD]);
    let mut isolate = platform.new_isolate();
    let script = platform.compile(format!("<?php $n = {n};\n{body}").as_bytes(), "t.php").expect("compiles");
    let mut host = Capture::default();
    let before = get(c);
    let status = isolate.new_context(&mut host).run(&script);
    let done = get(c) - before;
    assert_eq!(status, 0, "{}", String::from_utf8_lossy(&host.0));
    (String::from_utf8_lossy(&host.0).into_owned(), done)
}

/// The output at n = 20, after checking that the `c` done is the same at
/// n = 20 and n = 120. A run at n = 1 first makes what a thread makes once.
fn flat(body: &str, c: Count) -> String {
    run(body, 1, c);
    let (a, b) = (run(body, 20, c), run(body, 120, c));
    assert_eq!(b.1, a.1, "{c:?} grew with the iterations: {} at n = 20, {} at n = 120", a.1, b.1);
    a.0
}

/// `[]` is one shared empty array, as PHP's immutable empty array: an
/// empty array assigned, passed, returned or tested is not made and freed
/// each time; one written to is separated first, so another `[]` (and the
/// array it was copied from) stays empty.
#[test]
fn an_empty_array_literal_is_shared() {
    let out = flat(
        "function none($a = []) { return []; }
         function filled() { $a = []; $a[] = 1; $a['k'] = 2; return $a; }
         $s = 0;
         for ($i = 0; $i < $n; $i++) { $x = []; $y = none([]); $s += count($x) + count($y) + ($x === [] ? 1 : 0); }
         $p = filled(); $q = filled(); $q[] = 3; $e = [];
         echo $s / $n, ' ', count($p), count($q), count($e), count([]), ' ', implode(',', array_keys($q)), ' ', json_encode($e);",
        Count::ArrayFreed,
    );
    assert_eq!(out, "1 2300 0,k,1 []");
}

/// array_diff()/array_intersect() compare a value with no string of its
/// own (an array) as "Array": PHP's known string, one per thread here, not
/// one made per value compared.
#[test]
fn array_diff_of_arrays_makes_no_text_per_value() {
    let out = flat(
        "$a = [[1], [2], 'x']; $b = [[3]]; $s = 0;
         for ($i = 0; $i < $n; $i++) { $s += count(@array_diff($a, $b)) + count(@array_intersect($a, $b)); }
         echo $s / $n, ' ', implode(',', array_keys(@array_diff($a, $b)));",
        Count::StrMade,
    );
    assert_eq!(out, "3 2");
}

/// The compiled texts (names, literals) a file of `n` calls makes.
fn texts_compiling(n: usize) -> u64 {
    let platform = p8::Platform::new(&[p8::extensions::STANDARD]);
    let calls = "f(1); Foo(2); strlen($x); \\strlen($x); m\\g(); m\\G(); echo PHP_EOL, \\PHP_EOL, M\\C;\n".repeat(n);
    let source = format!("<?php namespace m; const C = 1; function f($a) {{}} function foo($a) {{}} function g() {{}} $x = 'ab';\n{calls}");
    let before = get(Count::TextMade);
    platform.compile(source.as_bytes(), "t.php").expect("compiles");
    get(Count::TextMade) - before
}

/// A function or constant named again in the same function is the same
/// call site: its texts (the name as written, lowercase, the global name
/// to fall back to) are made once, not made and dropped at each call.
#[test]
fn a_call_site_named_again_makes_no_texts() {
    texts_compiling(1);
    let (a, b) = (texts_compiling(20), texts_compiling(120));
    assert_eq!(b, a, "texts made grew with the calls: {a} for 20 lines of calls, {b} for 120");
}

/// A class named to reflection is looked up by its bytes as they are (a
/// name may hold any byte from 0x80 up, as PHP's names may): not through
/// a UTF-8 copy, which replaced such bytes and named another class.
#[test]
fn reflection_looks_a_class_up_by_its_bytes() {
    let (out, _) = run(
        "class Foo {} class_alias('Foo', \"F\\xffo\");
         echo (new ReflectionClass(\"F\\xffo\"))->getName(), ' ', class_exists(\"F\\xffo\") ? 'y' : 'n';
         try { new ReflectionClass('Nope'); } catch (ReflectionException $e) { echo ' ', $e->getMessage(); }",
        1,
        Count::StrMade,
    );
    assert_eq!(out, "Foo y Class \"Nope\" does not exist");
}
