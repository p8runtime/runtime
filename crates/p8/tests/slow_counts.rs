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

/// Two calls of one method name in one function, on objects of two
/// classes (each call its own cache entry), a receiver bound by `global`,
/// and variables sent by reference to a function and a builtin: inline.
#[test]
fn method_call_sites_and_sends_by_reference_stay_in_the_loop() {
    let out = flat(
        "class F { function n($x) { return 1; } }
         class G { function n($x) { return 2; } }
         class H { public $f; public $g; function both() { return $this->f->n(1) + $this->g->n(2); } }
         $h = new H; $h->f = new F; $h->g = new G;
         $GLOBALS['cache'] = new G;
         function via_global($k) { global $cache; return $cache->n($k); }
         function inc(&$x) { $x++; }
         $t = 0; $c = 0; $list = [];
         for ($i = 0; $i < $n; $i++) { $t += $h->both() + via_global($i); inc($c); array_push($list, $i); }
         echo $t, ' ', $c, ' ', count($list);",
    );
    assert_eq!(out, "100 20 20");
}

/// `&$o->p` (a property sent to a method's by-reference parameter, a
/// private one through `$this`) on declared properties: inline once the
/// name's cache entry is filled. (Binding one to a variable, `$r =
/// &$o->p`, and `foreach (.. as &$v)` are other instructions, `step`'s.)
#[test]
fn references_to_properties_stay_in_the_loop() {
    let out = flat(
        "class R {
             public $n = 0; private $own = 0;
             function add(&$x, $by) { $x += $by; }
             function mine() { $this->add($this->own, 1); return $this->own; }
         }
         $o = new R; $t = 0;
         for ($i = 0; $i < $n; $i++) { $o->add($o->n, 2); $t = $o->mine(); }
         echo $o->n, ' ', $t;",
    );
    assert_eq!(out, "40 20");
}

/// A parameter's default (an expression: its code runs when the argument
/// is not passed), passed and not: inline.
#[test]
fn parameter_defaults_stay_in_the_loop() {
    let out = flat(
        "const BASE = 10;
         function d($a, $b = BASE + 1, $c = [1, 2]) { return $a + $b + count($c); }
         $t = 0;
         for ($i = 0; $i < $n; $i++) { $t += d(1) + d(1, 2) + d(1, 2, [3]); }
         echo $t / $n;",
    );
    // (1 + 11 + 2) + (1 + 2 + 2) + (1 + 2 + 1).
    assert_eq!(out, "23");
}

/// `C::$x = v`, `static::$x = v` and `self::$x = v` (a variable's value,
/// a literal, a property bound by reference), and `C::m($a['k'])`
/// asking whether the argument goes by reference: inline once the
/// caches are filled.
#[test]
fn static_property_writes_and_static_arguments_stay_in_the_loop() {
    let out = flat(
        "class S {
             public static $n = 0; public static $last = null; public static $r = 0;
             static function put($v) { static::$last = $v; self::$n = self::$n + 1; S::$r = 5; return $v; }
         }
         $x = 7; $bound = &S::$r; $a = ['k' => 2]; $t = 0;
         for ($i = 0; $i < $n; $i++) { S::$last = $x; $t += S::put($a['k']); S::$n = S::$n; }
         echo S::$n, ' ', S::$last, ' ', $bound, ' ', $t / $n;",
    );
    assert_eq!(out, "20 2 5 2");
}

/// Native methods (ArrayIterator, ReflectionClass), one call site seeing
/// three classes in turn (a megamorphic site: the lookup by name on each
/// miss), and its sends and by-reference tests on a miss: in the loop's
/// arms (MethodCall's out-of-line part on the arm's operands), not the
/// detour.
#[test]
fn native_methods_and_megamorphic_sites_stay_in_the_loop() {
    let out = flat(
        "class A { function v(&$x) { $x++; return 1; } function w($x) { return $x; } }
         class B { function v($x) { return 2; } function w($x) { return $x + 1; } }
         class C { function v(&$x) { $x += 2; return 3; } function w($x) { return $x + 2; } }
         $objs = [new A, new B, new C]; $it = new ArrayIterator([1, 2, 3]); $rc = new ReflectionClass('A');
         $t = 0; $c = 0; $names = ''; $arr = ['k' => 0];
         for ($i = 0; $i < $n; $i++) {
             $o = $objs[$i % 3];
             $t += $o->v($c) + $it->count() + $o->w($arr['k']);
             $names = $rc->getName();
         }
         echo $t, ' ', $c, ' ', $names;",
    );
    // A 7 times, B 7, C 6: (7 + 14 + 18) + 20 * 3 + (7 + 12); $c 7 + 2 * 6.
    assert_eq!(out, "118 19 A");
}

/// `foreach` over an Iterator, an IteratorAggregate and a plain object's
/// properties: in the loop's arms (their methods called
/// out of line on the arm's operands), not the detour.
#[test]
fn foreach_over_objects_stays_in_the_loop() {
    let out = flat(
        "class It implements Iterator {
             private $i = 0;
             function __construct(private array $a) {}
             function rewind(): void { $this->i = 0; }
             function valid(): bool { return $this->i < count($this->a); }
             function current(): mixed { return $this->a[$this->i]; }
             function key(): mixed { return $this->i; }
             function next(): void { $this->i++; }
         }
         class Agg implements IteratorAggregate { function getIterator(): Iterator { return new ArrayIterator([4, 5]); } }
         class Pt { public $x = 1; public $y = 2; }
         $it = new It([1, 2, 3]); $agg = new Agg; $pt = new Pt; $t = 0; $keys = '';
         for ($i = 0; $i < $n; $i++) {
             foreach ($it as $k => $v) { $t += $k + $v; }
             foreach ($agg as $v) { $t += $v; }
             foreach ($pt as $k => $v) { $t += $v; $keys = $k; }
         }
         echo $t / $n, ' ', $keys;",
    );
    // (0+1 + 1+2 + 2+3) + (4+5) + (1+2). (A generator's are its own
    // instructions, Yield's and its return's: `step`'s.)
    assert_eq!(out, "21 y");
}

/// `new` of a class with a native constructor (ArrayObject), closures
/// called through a variable (`$f($x)`, one with `use` values, an arrow
/// function), a builtin's name called through a variable: in the loop's
/// arms (a closure's frame pushed there; the rest out of line on the
/// arm's operands), not the detour.
#[test]
fn native_constructors_and_calls_of_values_stay_in_the_loop() {
    let out = flat(
        "$k = 10; $add = function ($x) use ($k) { return $x + $k; }; $inc = fn ($x) => $x + 1; $len = 'strlen';
         $t = 0;
         for ($i = 0; $i < $n; $i++) {
             $ao = new ArrayObject([1, 2]);
             $t += count($ao) + $add(1) + $inc(2) + $len('abc');
         }
         echo $t / $n;",
    );
    // 2 + 11 + 3 + 3.
    assert_eq!(out, "19");
}

/// `$list[] = v` and `$a['k'][] = v` on variables bound by reference
/// (`global`, `&`), `@expr`, `-$x`, a closure's `use ($x)` and `use
/// (&$x)`, `new static`, `f(...$args)`, and `$this->s .= 'x'`, `$o->n
/// -= 1` on declared properties: in the loop's arms (the spread call out
/// of line on the arm's operands), not the detour.
#[test]
fn appends_through_references_silence_negation_captures_and_spreads_stay_in_the_loop() {
    let out = flat(
        "$GLOBALS['list'] = [];
         function add($x) { global $list; $list[] = $x; }
         class M { static function make() { return new static; } }
         class N extends M {}
         function sum3($a, $b, $c) { return $a + $b + $c; }
         class Acc { public $s = ''; public int $n = 0; function add() { $this->s .= 'x'; $this->n += 2; } }
         $b = ['k' => []]; $a = &$b; $t = 0; $f = 1.5; $args = [1, 2, 3]; $k = 2; $acc = new Acc;
         for ($i = 0; $i < $n; $i++) {
             $acc->add();
             $acc->n -= 1;
             add($i);
             $a['k'][] = $i;
             $t += @strlen('ab') + (-$i) + (-$f);
             $g = function ($x) use ($k, &$t) { return $x * $k; };
             $t += $g(1);
             $o = N::make();
             $t += sum3(...$args);
         }
         echo count($GLOBALS['list']), ' ', count($b['k']), ' ', $t, ' ', get_class($o), ' ', strlen($acc->s), ' ', $acc->n;",
    );
    // $t: the sum of 2 - $i - 1.5 + 2 + 6.
    assert_eq!(out, "20 20 -20 N 20 20");
}

/// `[$a, $b] = ..` and `$a[1]` (of elements bound by reference too, of
/// an array and of an array held through a reference), `['k' => $v] =
/// ..`, a function's and a method's `static $x` (stepped through the
/// reference), `foreach ($a as $k => &$v)` over an array, and `f($a['k'])`
/// of a function declared at run time asking whether it takes it by
/// reference: inline. (Making
/// `$a['k']` a reference for one that does, and `unset($v)`, are other
/// instructions, `step`'s.)
#[test]
fn lists_static_variables_and_by_reference_foreach_stay_in_the_loop() {
    let out = flat(
        "function counter() { static $c = 0; return ++$c; }
         class S { function m() { static $m = 100; return $m++; } }
         if (true) { function plain($x) { return $x; } }
         $s = new S; $arr = [1, 2, 3]; $map = ['k' => 5, 'j' => 6]; $d = ['k' => 4]; $t = 0; $one = 1; $bound = [&$one, 2];
         for ($i = 0; $i < $n; $i++) {
             [$a, $b] = $arr;
             ['k' => $v, 'j' => $w] = $map;
             $t += $a + $b + $v + $w + counter() + $s->m();
             foreach ($arr as $key => &$e) { $e += $key; }
             $t += plain($d['k']) + $arr[1] + $bound[0] - 1;
         }
         echo $t, ' ', implode(',', $arr), ' ', $d['k'];",
    );
    assert_eq!(out, "3200 1,22,43 4");
}

/// An array literal of 300 elements made at run time (#456): each
/// element's temporaries go back once it is added, so the function stays
/// inside the 256 registers an instruction names, and its instructions
/// run in the loop (a function past them runs every one on `step`).
#[test]
fn a_wide_array_literal_stays_in_the_loop() {
    let items: Vec<String> = (0..300).map(|k| format!("'k{k}' => $x + {k}")).collect();
    let body = format!(
        "function wide($x) {{ return [{}]; }}
         $t = 0;
         for ($i = 0; $i < $n; $i++) {{ $a = wide($i); $t += $a['k0'] + $a['k299']; }}
         echo count($a), ' ', $t;",
        items.join(", ")
    );
    let out = flat(&body);
    assert_eq!(out, "300 6360");
}

/// A base class's methods on objects of three subclasses in turn (each
/// property access's entry filled again for the class it meets), its
/// private and protected constants through `self::`, `isset($this->v)`,
/// `$o->$name` read, written and asked by isset, and `isset($a[k1][k2]..)`:
/// inline.
#[test]
fn property_misses_class_constants_and_isset_stay_in_the_loop() {
    let out = flat(
        "class B0 {
             public $v = 1; private const P = 2; protected const Q = 3;
             function get() { return $this->v + self::P + self::Q; }
             function set($x) { $this->v = $x; }
             function has() { return isset($this->v); }
         }
         class B1 extends B0 {} class B2 extends B0 {} class B3 extends B0 {}
         $objs = [new B1, new B2, new B3]; $t = 0; $name = 'v'; $a = ['x' => ['y' => ['z' => 1]]];
         for ($i = 0; $i < $n; $i++) {
             $o = $objs[$i % 3];
             $o->set($i);
             $t += $o->get() + ($o->has() ? 1 : 0);
             $o->$name = 5;
             $t += $o->$name + (isset($o->$name) ? 1 : 0) + (isset($a['x']['y']['z']) ? 1 : 0) + (isset($a['x']['n']) ? 10 : 0);
         }
         echo $t;",
    );
    // The sum of $i + 2 + 3 + 1 + 5 + 1 + 1.
    assert_eq!(out, "450");
}

/// A generator gone through (its call, `yield`, `yield from`, its return),
/// `new $class`, `new Exception`, a native static method, `f(...)`,
/// references to an element, from an element, to a static property and
/// to a variable, `S::$n += 1`, `unset($x)`, an include that fails, a
/// string key written through a variable bound by reference and a string
/// written at an int key:
/// in the loop's arms (out of line on their operands), not the detour.
#[test]
fn generators_references_static_ops_and_includes_stay_in_the_loop() {
    let out = flat(
        "class S { public static $n = 0; }
         function gen($k) { yield $k; yield from [$k + 1, $k + 2]; }
         $cls = 'stdClass'; $t = 0; $a = ['k' => 0]; $x = 1; $m = 0; $bb = []; $b = &$bb; $ints = [0, 0];
         for ($i = 0; $i < $n; $i++) {
             $b['s'] = $i;
             $ints[1] = 'str';
             foreach (gen($i) as $v) { $t += $v; }
             $o = new $cls;
             $e = new Exception('e');
             $f = Closure::fromCallable('strlen');
             $g = strlen(...);
             $t += $f('ab') + $g('abc');
             $r = &$a['k']; $r++; unset($r);
             $a['j'] = &$x;
             $s = &S::$n; $s++; unset($s);
             S::$n += 1;
             $y = &$m; $y++; unset($y);
             $tmp = 1; unset($tmp);
             $inc = @include 'no-such-file-for-the-test.php';
         }
         echo $t, ' ', $a['k'], ' ', S::$n, ' ', $m, ' ', get_class($o), ' ', var_export($inc, true);",
    );
    assert_eq!(out, "730 20 40 20 stdClass false");
}

/// `$this->list[k] = v` from a base class's method on two subclasses, a
/// stdClass's dynamic property read (by name and `$o->$name`) and asked
/// by isset, isset of an element of null and of a string's offsets, an
/// enum case, a returned array bound by reference, writes to elements
/// bound by reference (an int key, a string key, a path), `(object)`,
/// `$$name`, `$_SERVER` in a function, `[&$x]`, and `$f($a['k'])` of a callable name
/// asking whether it takes the element by reference: inline (the cast,
/// `$$name`, the superglobal and the callable's question out of line on
/// their arms' operands).
#[test]
fn dynamic_properties_references_and_cases_stay_in_the_loop() {
    let out = flat(
        "enum E { case A; case B; }
         class P0 { public $list = []; function put($k) { $this->list[$k] = $k; return count($this->list); } }
         class P1 extends P0 {} class P2 extends P0 {}
         function addm(array &$m) { $m[] = 1; }
         function rr(): array { $m = []; addm($m); return $m; }
         function sv() { return $_SERVER === null || is_array($_SERVER) ? 1 : 0; }
         $ps = [new P1, new P2]; $std = new stdClass; $std->dyn = 2; $dn = 'dyn'; $nul = null; $str = 'abc';
         $re = [0]; $rx = &$re[0]; $rs = ['k' => 0]; $ry = &$rs['k']; $rp = ['a' => [0]]; $rz = &$rp['a'][0];
         $t = 0; $e = null; $vv = 'str'; $x0 = 0; $cb = 'strlen'; $ca = ['s' => 'abcd'];
         for ($i = 0; $i < $n; $i++) {
             $ob = (object) ['p' => 1];
             $t += $ob->p + strlen($$vv) + sv();
             $lit = [&$x0];
             $t += $cb($ca['s']);
             $t += $ps[$i % 2]->put($i % 4);
             $t += $std->dyn + $std->$dn + (isset($std->dyn) ? 1 : 0);
             $t += (isset($nul['k']) ? 1 : 0) + (isset($str[1]) ? 1 : 0) + (isset($str[5]) ? 1 : 0);
             $e = E::B;
             $t += count(rr());
             $re[0] = $i; $rs['k'] = $i; $rp['a'][0] = $i;
         }
         echo $t, ' ', $rx + $ry + $rz, ' ', $e->name;",
    );
    assert_eq!(out, "358 57 B");
}

/// A class and a function declared where the code runs, then used: no
/// instruction of the run goes to the detour but the script's end, as for
/// a script with nothing in it (each declaration runs once, so this one
/// compares with that instead of comparing two sizes).
#[test]
fn declarations_run_in_the_loop() {
    let (_, empty) = run("", 1);
    let (out, slow) = run(
        "if ($n > 0) { class Late { function m() { return 'm'; } } function late() { return 'f'; } }
         $o = new Late;
         $s = late() . $o->m();
         if ($s !== 'fm') { throw new Exception($s); }",
        1,
    );
    assert_eq!((out.as_str(), slow), ("", empty));
}

/// A variable sent to a function the call site has not resolved yet (one
/// declared where the code runs, as an included file's): out of line on
/// the arm's operands, which resolve the site, not the detour; compared
/// with an empty script, as above.
#[test]
fn a_first_call_sends_its_variable_in_the_loop() {
    let (_, empty) = run("", 1);
    let (out, slow) = run(
        "if ($n > 0) { function late2($x, &$y) { $y = $x; return $x; } }
         $v = 'f'; $w = null;
         if (late2($v, $w) !== 'f' || $w !== 'f') { throw new Exception($v); }",
        1,
    );
    assert_eq!((out.as_str(), slow), ("", empty));
}

/// `$this->p[$k] = v` on a property `next($this->p[$k])` bound by
/// reference (WP_Hook::apply_filters), `$this->class::$m()` of a class
/// name in a property, a property read of an object a `global` holds (a
/// class with `__get`), `$a['x'][$k] = true` through a by-reference
/// parameter (a literal value), a string's offset through a reference and
/// `empty()` of a reference's missing element, `(bool)` and `(array)` of
/// an object, `global` in the main script, a variable sent undefined:
/// inline, or out of line on the arms' operands, not the detour.
#[test]
fn reference_paths_class_names_casts_and_globals_stay_in_the_loop() {
    let out = flat(
        "class H {
             private $it = []; private $pr = [10 => 1, 20 => 2];
             function apply($v) {
                 $l = 0;
                 $this->it[$l] = $this->pr;
                 do { $v += current($this->it[$l]); } while (false !== next($this->it[$l]));
                 unset($this->it[$l]);
                 return $v;
             }
         }
         class M { public $declared = 3; function __get($n) { return 7; } }
         class Fac { static function make($x) { return $x * 2; } }
         class Holder { public $cls = 'Fac'; function call($m, $x) { return $this->cls::$m($x); } }
         function g() { global $gm; return $gm->declared; }
         function setk(array &$a, $k) { $a['x'][$k] = true; }
         function first(&$s) { return $s[0]; }
         function quiet(array &$r) { return empty($r['missing']) ? 1 : 0; }
         function byval($x) { return $x === null ? 1 : 0; }
         $gm = new M; $h = new H; $ho = new Holder; $arr = []; $str = 'hello'; $t = 0; $o = new M;
         for ($i = 0; $i < $n; $i++) {
             global $mainvar;
             $t += $h->apply(0) + g() + $ho->call('make', 1);
             setk($arr, $i % 3);
             $t += ord(first($str)) - 100 + quiet($arr);
             $t += ((bool) $o ? 1 : 0) + count((array) $o);
             $t += @byval($undefined);
         }
         echo $t, ' ', count($arr['x']), ' ', var_export($mainvar, true);",
    );
    assert_eq!(out, "320 3 NULL");
}
