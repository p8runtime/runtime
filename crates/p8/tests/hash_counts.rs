//! Hash-table work (#411): where PHP appends to a list, builds a record from
//! keys it knows, or holds a pointer, p8 does no keyed insert, hashes no key
//! and looks up no name. Each test runs a loop at two sizes: the counted
//! work must not grow with the iterations (it may be a fixed amount: the
//! script's own literals), and the output must be PHP's. Counted per thread
//! by p8_runtime::counts, which this crate's tests turn on.

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
    (String::from_utf8(host.0).expect("utf-8"), done)
}

/// The output at n = 20, after checking that the `c` done is the same at
/// n = 20 and n = 120. A run at n = 1 first makes what a thread makes once
/// (a builtin's keys).
fn flat(body: &str, c: Count) -> String {
    run(body, 1, c);
    let (a, b) = (run(body, 20, c), run(body, 120, c));
    assert_eq!(b.1, a.1, "{c:?} grew with the iterations: {} at n = 20, {} at n = 120", a.1, b.1);
    a.0
}

/// `$a[] = v` and array_merge() of lists append (ZEND_HASH_FILL_PACKED):
/// no keyed insert per element once the list has a buffer of its own.
#[test]
fn appending_to_a_list_is_not_a_keyed_insert() {
    let out = flat(
        "$a = []; for ($i = 0; $i < $n; $i++) { $a[] = $i; }
         $m = array_merge($a, $a, [1, 2]);
         echo count($a), ' ', count($m), ' ', $m[$n], ' ', array_is_list($m) ? 'list' : 'map', ' ', array_key_last($m);
         $m[] = 'x'; echo ' ', array_key_last($m);",
        Count::Insert,
    );
    assert_eq!(out, "20 42 0 list 41 42");
}
