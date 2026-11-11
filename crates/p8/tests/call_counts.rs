//! The work a call does (#413), counted per thread by p8_runtime::counts,
//! which this crate's tests turn on (see hash_counts.rs). Each test runs a
//! loop at two sizes: the counted work must not grow with the iterations
//! (a fixed number may: the first call from a depth, the first lookup that
//! fills a cache), and the output must be PHP's.

use p8_runtime::counts::{Count, get};

#[derive(Default)]
struct Capture(Vec<u8>);

impl p8::Host for Capture {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

/// `body` with `$n` set to `n`, as one request: its output, and how many
/// of `count` it did.
fn run(body: &str, n: u32, count: Count) -> (String, u64) {
    let platform = p8::Platform::new(&[p8::extensions::STANDARD]);
    let mut isolate = platform.new_isolate();
    let script = platform.compile(format!("<?php $n = {n};\n{body}").as_bytes(), "t.php").expect("compiles");
    let mut host = Capture::default();
    let before = get(count);
    let status = isolate.new_context(&mut host).run(&script);
    let done = get(count) - before;
    assert_eq!(status, 0, "{}", String::from_utf8_lossy(&host.0));
    (String::from_utf8_lossy(&host.0).into_owned(), done)
}

/// The output at n = 20, after checking that `count` is the same at
/// n = 20 as at n = 120.
fn flat(body: &str, count: Count) -> String {
    let (a, b) = (run(body, 20, count), run(body, 120, count));
    assert_eq!(b.1, a.1, "{count:?} grew with the iterations: {} at n = 20, {} at n = 120", a.1, b.1);
    a.0
}

/// `$o->m(..)` called again and again from one depth (by value, and
/// sending a variable by reference): the call site's code is not made a
/// strong handle again for each call, the frame slot already holds it.
#[test]
fn a_method_called_again_from_the_same_depth_takes_no_count_of_its_code() {
    let out = flat(
        "class A {
             public $t = 0;
             function add($x) { $this->t += $x; return $x; }
             function by(&$r) { $r++; return 1; }
         }
         $o = new A; $v = 0; $s = 0;
         for ($i = 0; $i < $n; $i++) { $s += $o->add($i); }
         for ($i = 0; $i < $n; $i++) { $s += $o->by($v); }
         echo $s, ' ', $v, ' ', $o->t;",
        Count::CodeCounted,
    );
    assert_eq!(out, "210 20 190");
}

/// assert() of an assertion that holds is answered from its argument:
/// the registers are not lent to it (they are for one that fails, which
/// may run assert.callback).
#[test]
fn a_holding_assert_is_not_lent_the_registers() {
    let out = flat(
        "$t = 0;
         for ($i = 0; $i < $n; $i++) { assert($i >= 0); assert(true, 'holds'); $t += assert(1) ? 1 : 0; }
         try { assert($n < 0); } catch (AssertionError $e) { echo get_class($e), ': ', $e->getMessage(), ' '; }
         echo $t;",
        Count::BuiltinLent,
    );
    assert_eq!(out, "AssertionError: assert($n < 0) 20");
}

/// A builtin that calls back into PHP (array_map, array_filter, usort),
/// called again and again: its entry for traces is the last call's, used
/// again, not a new one each call; and a trace from its callback shows it.
#[test]
fn a_builtin_calling_back_uses_its_trace_entry_again() {
    let out = flat(
        "$t = 0; $shown = '';
         for ($i = 0; $i < $n; $i++) {
             $t += array_sum(array_map(fn ($x) => $x * 2, [1, $i]));
             $t += count(array_filter([0, $i, 2], fn ($x) => $x > 0));
             $a = [3, 1, 2]; usort($a, fn ($x, $y) => $x <=> $y); $t += $a[0];
             $shown = array_map(fn () => debug_backtrace()[1]['function'] . '/' . count(debug_backtrace()[1]['args']), [1])[0];
         }
         echo $t, ' ', $shown;",
        Count::NativeEntry,
    );
    assert_eq!(out, "479 array_map/2");
}

/// foreach over an Iterator: `valid`, `current`, `key` and `next` are
/// looked up by name once (then found as a call site finds them), not at
/// every element; from a method of another class too.
#[test]
fn foreach_over_an_iterator_looks_its_methods_up_once() {
    let out = flat(
        "class It implements Iterator {
             private $i = 0;
             function __construct(private int $n) {}
             function current(): mixed { return $this->i * 2; }
             function key(): mixed { return $this->i; }
             function next(): void { $this->i++; }
             function rewind(): void { $this->i = 0; }
             function valid(): bool { return $this->i < $this->n; }
         }
         class Walk { function sum($it) { $t = 0; foreach ($it as $k => $v) { $t += $v - $k; } return $t; } }
         $t = 0;
         foreach (new It($n) as $k => $v) { $t += $v - $k; }
         echo $t, ' ', (new Walk)->sum(new It($n));",
        Count::MethodLookup,
    );
    assert_eq!(out, "190 190");
}
