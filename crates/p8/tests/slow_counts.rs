//! The interpreter loop's own cases (#375): an instruction the loop runs
//! inline does not go out of line (`step`, or a generated instruction's
//! cold method). Each test runs a loop at two sizes: the instructions run
//! out of line must not grow with the iterations (a fixed number may: a
//! call site's first call, which fills its cache), and the output must be
//! PHP's. Counted per thread by p8_runtime::counts (`Count::Slow`), which
//! this crate's tests turn on (see hash_counts.rs).

use p8_runtime::counts::{Count, get};

#[derive(Default)]
struct Capture(Vec<u8>);

impl p8::Host for Capture {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

/// `body` with `$n` set to `n`, as one request: its output, and how many
/// instructions ran out of the loop.
fn run(body: &str, n: u32) -> (String, u64) {
    let platform = p8::Platform::new(&[p8::extensions::STANDARD]);
    let mut isolate = platform.new_isolate();
    let script = platform.compile(format!("<?php $n = {n};\n{body}").as_bytes(), "t.php").expect("compiles");
    let mut host = Capture::default();
    let before = get(Count::Slow);
    let status = isolate.new_context(&mut host).run(&script);
    let done = get(Count::Slow) - before;
    assert_eq!(status, 0, "{}", String::from_utf8_lossy(&host.0));
    (String::from_utf8_lossy(&host.0).into_owned(), done)
}

/// The output at n = 20, after checking that the instructions run out of
/// the loop are as many at n = 20 as at n = 120.
fn flat(body: &str) -> String {
    let (a, b) = (run(body, 20), run(body, 120));
    assert_eq!(b.1, a.1, "the slow path grew with the iterations: {} at n = 20, {} at n = 120", a.1, b.1);
    a.0
}

/// A string's offset (`$s[$i]`, `$s[-1]`, out of range under `??`), and
/// `??` and empty() of a dimension of null or of a missing key: inline.
#[test]
fn string_offsets_and_quiet_reads_of_null_stay_in_the_loop() {
    let out = flat(
        "$s = 'hello'; $nul = null; $a = ['x' => 1]; $t = 0; $c = ''; $last = -1;
         for ($i = 0; $i < $n; $i++) {
             $c = $s[$i % 5] . $s[$last];
             $t += ($nul['k'] ?? 2) + (empty($a['nope']) ? 1 : 0) + (empty($nul['k']) ? 1 : 0) + ($s[10] ?? 3) + (empty($a['x']) ? 10 : 0);
         }
         echo $c, ' ', $t / $n;",
    );
    assert_eq!(out, "oo 7");
}

/// `C::f($x)` (the call sends `$x` itself: by reference here), a typed
/// parameter holding a reference, a typed variadic, the variadic's list:
/// inline. And `$o->f($x)` sending `$x` by reference.
#[test]
fn static_calls_sending_variables_and_variadics_stay_in_the_loop() {
    let out = flat(
        "class A {
             static function first(array &$a, int ...$xs) { return $a[0] + count($xs); }
             function plus(array &$a, int $by) { return $a[0] + $by; }
         }
         $arr = [5]; $o = new A; $t = 0;
         for ($i = 0; $i < $n; $i++) { $t += A::first($arr, 1, 2) + $o->plus($arr, 1); }
         echo $arr[0], ' ', $t;",
    );
    assert_eq!(out, "5 260");
}

/// A closure made (`function () {}`, `fn () =>`), `$o::class`, and
/// `[...$a, ...$b]` of arrays: inline.
#[test]
fn closures_class_names_and_unpacking_stay_in_the_loop() {
    let out = flat(
        "class P { function make() { return fn () => 1; } }
         $p = new P; $list = [1, 2]; $map = ['k' => 'v']; $t = 0; $name = '';
         for ($i = 0; $i < $n; $i++) {
             $f = function () { return 2; };
             $g = $p->make();
             $name = $p::class;
             $u = [...$list, ...$map, ...$list];
             $t += count($u);
         }
         echo get_class($f), ' ', get_class($g), ' ', $name, ' ', $t / $n, ' ', implode(',', array_keys($u));",
    );
    assert_eq!(out, "Closure Closure P 5 0,1,k,2,3");
}
