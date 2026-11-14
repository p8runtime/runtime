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

/// `C::m(..)`, `self::m(..)` and `static::m(..)` called again and again
/// from one depth take their code as `$o->m(..)` does: no strong handle
/// made for each call when the frame slot already holds it, one made when
/// it holds another's (two methods called in turn).
#[test]
fn a_static_method_called_again_from_the_same_depth_takes_no_count_of_its_code() {
    let out = flat(
        "class A {
             static function add($x) { return $x + 1; }
             static function none() { return 1; }
             static function run($n) {
                 $s = 0;
                 for ($i = 0; $i < $n; $i++) { $s += self::add($i); }
                 for ($i = 0; $i < $n; $i++) { $s += static::none(); }
                 return $s;
             }
         }
         $s = 0;
         for ($i = 0; $i < $n; $i++) { $s += A::add($i); }
         for ($i = 0; $i < $n; $i++) { $s += A::none(); }
         echo $s + A::run($n);",
        Count::CodeCounted,
    );
    assert_eq!(out, "460");
    let turns = "class A {
             static function a($x) { return $x; }
             static function b($x) { return 1; }
         }
         $s = 0;
         for ($i = 0; $i < $n; $i++) { $s += A::a($i); $s += A::b($i); }
         echo $s;";
    let (few, many) = (run(turns, 20, Count::CodeCounted), run(turns, 120, Count::CodeCounted));
    assert_eq!(few.0, "210");
    assert_eq!(many.1 - few.1, 200, "two static methods in turn: one handle a call, {} at n = 20, {} at n = 120", few.1, many.1);
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

/// serialize() of a value with no object in it is answered from its
/// argument: the registers are not lent to it. One with an object deep in
/// it (whose __serialize is PHP code) is lent them, and that PHP code
/// runs once a call.
#[test]
fn serialize_of_a_value_with_no_object_is_not_lent_the_registers() {
    let plain = flat(
        "$a = ['id' => 7, 'name' => 'post', 'ok' => true, 'f' => 1.5, -3 => [null, [&$n]]];
         $t = 0;
         for ($i = 0; $i < $n; $i++) { $t += strlen(serialize($a)); $t += strlen(serialize($i)); }
         echo serialize($a), ' ', $t;",
        Count::BuiltinLent,
    );
    assert_eq!(plain, "a:5:{s:2:\"id\";i:7;s:4:\"name\";s:4:\"post\";s:2:\"ok\";b:1;s:1:\"f\";d:1.5;i:-3;a:2:{i:0;N;i:1;a:1:{i:0;i:20;}}} 2170");
    let object = "class M { function __serialize(): array { echo 's'; return [1]; } }
         $a = [1, [2, new M]];
         for ($i = 0; $i < $n; $i++) { serialize($a); }
         echo ' ', serialize($a);";
    let (few, many) = (run(object, 20, Count::BuiltinLent), run(object, 120, Count::BuiltinLent));
    assert_eq!(few.0, format!("{} s{}", "s".repeat(20), "a:2:{i:0;i:1;i:1;a:2:{i:0;i:2;i:1;O:1:\"M\":1:{i:0;i:1;}}}"));
    assert_eq!(many.1 - few.1, 100, "an object in it: lent each call, {} at n = 20, {} at n = 120", few.1, many.1);
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

/// call_user_func('add', ..) and call_user_func_array('add', [..]) called
/// again and again on one name (WordPress's hooks): the function is found
/// by its name once, not twice per call (the builtin's callable check and
/// its call each looked it up).
#[test]
fn a_function_called_by_name_again_and_again_is_looked_up_once() {
    let out = flat(
        "function add($a, $b) { return $a + $b; }
         $s = 0;
         for ($i = 0; $i < $n; $i++) { $s = call_user_func('add', $s, $i); }
         for ($i = 0; $i < $n; $i++) { $s = call_user_func_array('ADD', [$s, $i]); }
         echo $s;",
        Count::NameLookup,
    );
    assert_eq!(out, "380");
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
