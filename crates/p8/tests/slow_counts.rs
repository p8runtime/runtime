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

/// `global $x` in a function (a global of an included file's kind: not
/// the main script's variable), read and written: inline once its
/// first run cached the global's index.
#[test]
fn globals_bound_in_functions_stay_in_the_loop() {
    let out = flat(
        "$GLOBALS['g'] = 1; $GLOBALS['h'] = 'x';
         function bump() { global $g; $g++; return $g; }
         function both() { global $g, $h; return $g * 2 + ($h === 'x' ? 1 : 0); }
         $t = 0;
         for ($i = 0; $i < $n; $i++) { $t += bump(); $s = both(); }
         echo $t, ' ', $s;",
    );
    // n = 20: 2 + .. + 21, and 21 * 2 + 1.
    assert_eq!(out, "230 43");
}

/// `$a[k1][k2] = v` and three keys, on a variable and on a property
/// (`$this->p[k] = v`, `$o->p[k1][k2] = v`): inline.
#[test]
fn dimension_paths_stay_in_the_loop() {
    let out = flat(
        "class P { public $d = []; public $list; function put($k, $v) { $this->d[$k] = $v; $this->d['all'][$k] = 1; $this->list[] = $v; } }
         $o = new P; $a = []; $b = ['x' => ['y' => []]]; $c = [];
         for ($i = 0; $i < $n; $i++) {
             $a['k' . ($i % 3)]['n'] = $i;
             $b['x']['y'][$i % 4] = [$i];
             $o->put($i % 5, 'v');
             $o->d['two'][$i % 2] = $i;
             $o->d['seq'][] = $i;
             $c['k' . ($i % 2)][] = $i;
         }
         echo count($a), ' ', count($b['x']['y']), ' ', count($o->d), ' ', count($o->d['all']), ' ', implode(',', $o->d['two']), ' ', count($o->list), ' ', count($o->d['seq']), ' ', count($c['k1']);",
    );
    assert_eq!(out, "3 4 8 5 18,19 20 20 10");
}

/// `new C(...$args)` with a PHP constructor, positions only (a list, and
/// int keys out of order): inline.
#[test]
fn new_with_spread_arguments_stays_in_the_loop() {
    let out = flat(
        "class V { public $s; function __construct($a, $b = 0, ...$more) { $this->s = $a + $b + count($more); } }
         $t = 0; $args = [1, 2, 3, 4]; $odd = [3 => 5, 1 => 6];
         for ($i = 0; $i < $n; $i++) { $t += (new V(...$args))->s + (new V(...$odd))->s; }
         echo $t / $n;",
    );
    assert_eq!(out, "16");
}

/// `static::f($x, $y + 1)` (the method, so by-reference or not, known
/// only when it runs; an argument that is no variable, so each is sent
/// on its own, SendVarStatic): the sends inline.
#[test]
fn static_call_sends_stay_in_the_loop() {
    let out = flat(
        "class S {
             static function r(&$x, $d) { $x += $d; return $x; }
             static function v($x, $d) { return $x + $d; }
             static function go($y) { $z = $y; return static::r($z, $y * 0 + 1) + static::v($y, $y * 0 + 1); }
         }
         $t = 0;
         for ($i = 0; $i < $n; $i++) { $t += S::go($i); }
         echo $t;",
    );
    // n = 20: 2 * (1 + .. + 20).
    assert_eq!(out, "420");
}

/// `unset($a[k1][k2])` and `$a[k]++` / `--$a[k1][k2]` through arrays, on
/// a variable and through a reference (missing keys unset too): inline.
#[test]
fn unset_and_steps_of_dimensions_stay_in_the_loop() {
    let out = flat(
        "$a = ['x' => ['y' => 0]]; $c = [0, 0, 0, 0]; $g = ['k' => ['j' => 5]]; $r = &$g;
         for ($i = 0; $i < $n; $i++) {
             $a['x']['y']++;
             $k = 'z' . ($i % 3);
             $a['x'][$k] = $i;
             unset($a['x'][$k]);
             $c[$i % 4]++;
             --$c[$i % 4];
             $c[$i % 4]++;
             $b = [1, 2, 3];
             unset($b[1]);
             unset($b[7]);
             $r['k']['j']--;
             unset($r['k']['gone']);
         }
         echo $a['x']['y'], ' ', count($a['x']), ' ', implode(',', $c), ' ', implode(',', array_keys($b)), ' ', $g['k']['j'];",
    );
    assert_eq!(out, "20 1 5,5,5,5 0,2 -15");
}

/// `(int)`, `(string)`, `(bool)`, `(float)`, `(array)` of strings, ints,
/// floats, bools, null, arrays and a reference: inline.
#[test]
fn scalar_casts_stay_in_the_loop() {
    let out = flat(
        "$t = 0; $s = ''; $f = 0.0; $ref = 7; $rr = &$ref; $yes = true; $nul = null; $half = '0.5'; $list = [5, 6];
         for ($i = 0; $i < $n; $i++) {
             $t += (int) ('1' . ($i % 10)) + (int) ($i * 0.5) + (int) (bool) $i + (int) $rr;
             $s = (string) $i . (string) ($i * 0.25) . (string) $yes . (string) $nul;
             $f += (float) $i + (float) $half;
             $u = (array) $i;
             $e = (array) $nul;
             $l = (array) $list;
         }
         echo $t, ' ', $s, ' ', $f, ' ', implode(',', $u), count($e), implode(',', $l);",
    );
    assert_eq!(out, "539 194.751 200 1905,6");
}

/// `$o->p++`, `--$o->p`, `$this->p++` on declared properties (untyped,
/// typed int, private): inline once the name's cache entry is filled.
#[test]
fn steps_of_properties_stay_in_the_loop() {
    let out = flat(
        "class K {
             public $n = 0; public int $t = 0; private $p = 0;
             function bump() { $this->p++; ++$this->t; return $this->p; }
         }
         $o = new K;
         for ($i = 0; $i < $n; $i++) { $o->n++; $o->bump(); --$o->n; $o->n++; }
         echo $o->n, ' ', $o->t, ' ', $o->bump();",
    );
    assert_eq!(out, "20 20 21");
}

/// `$o->m($a['k'])` and `$o->m($o->p)` (whether the argument goes by
/// reference asked as the call runs; by value here: a by-reference one's
/// element or property reference is `step`'s): inline once the call
/// site's cache knows the method.
#[test]
fn method_arguments_asked_by_reference_stay_in_the_loop() {
    let out = flat(
        "class M { function v($x) { return $x; } }
         class P { public $p = 3; }
         $m = new M; $a = ['k' => 2]; $o = new P; $t = 0;
         for ($i = 0; $i < $n; $i++) { $t += $m->v($a['k']) + $m->v($o->p); }
         echo $t;",
    );
    assert_eq!(out, "100");
}
