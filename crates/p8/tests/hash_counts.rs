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

/// getrusage() and gc_status() return records whose keys are made once
/// per thread: no key hashed and no keyed insert per call.
#[test]
fn builtin_records_reuse_their_keys() {
    let body = "$s = 0; for ($i = 0; $i < $n; $i++) { $r = getrusage(); $g = gc_status(); $s += count($r) + count($g); }
         echo $s / $n, ' ', implode(',', array_keys($g)), ' ', isset($r['ru_utime.tv_sec']) ? 'y' : 'n', ' ', $g['threshold'];";
    let out = flat(body, Count::Hash);
    assert_eq!(
        out,
        "29 running,protected,full,runs,collected,threshold,buffer_size,roots,application_time,collector_time,destructor_time,free_time y 10001"
    );
    flat(body, Count::Insert);
}

/// A trace's entries (an exception's, debug_backtrace()'s) are built from
/// keys made once per thread and appended (PHP's known strings,
/// _zend_hash_append_ex): no key hashed and no keyed insert per frame.
#[test]
fn trace_entries_reuse_their_keys() {
    let body = "class C { function m($x) { return [(new Exception('e'))->getTrace(), debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS)]; } }
         function f($o, $i) { return $o->m($i); }
         $s = 0; for ($i = 0; $i < $n; $i++) { [$t, $b] = f(new C, $i); $s += count($t) + count($t[0]) + count($t[1]) + count($b[0]) + count($b[1]); }
         echo $s / $n; foreach ([$t[0], $t[1], $b[0], $b[1]] as $e) { echo ' ', implode(',', array_keys($e)); }";
    let out = flat(body, Count::Hash);
    assert_eq!(out, "20 file,line,function,class,type,args file,line,function,args file,line,function,class,type file,line,function");
    flat(body, Count::Insert);
}

/// A reflection object holds the class (and a method's class) it was made
/// for, as PHP's reflection_object holds the pointer: asking it again
/// looks no class up by name, also through a parameter it made.
#[test]
fn reflection_holds_its_class() {
    let out = flat(
        "class C { function m(int $a, $b = 2) {} }
         $rc = new ReflectionClass('C'); $rm = new ReflectionMethod('C', 'm'); $rp = $rm->getParameters()[1];
         $s = 0;
         for ($i = 0; $i < $n; $i++) {
             $s += strlen($rc->getName()) + strlen($rm->getName()) + $rm->getNumberOfParameters() + ($rm->isPublic() ? 1 : 0) + $rp->getPosition();
         }
         echo $s / $n, ' ', $rm->class, ' ', $rp->getName(), ' ', $rp->getDeclaringClass()->getName(), ' ', $rp->getDefaultValue();",
        Count::NameLookup,
    );
    assert_eq!(out, "6 C b C 2");
}

/// foreach and `yield from` over an Iterator or an IteratorAggregate
/// call rewind(), valid() and getIterator() by their fixed symbols, as
/// PHP's zend_user_it_* call them through the class's iterator
/// functions: no method name looked up per loop.
#[test]
fn iterating_an_object_looks_up_no_method_name() {
    let out = flat(
        "class Agg implements IteratorAggregate { function getIterator(): Iterator { return new ArrayIterator([1, 2]); } }
         class It implements Iterator { private $i = 0;
             function current(): mixed { return $this->i; } function key(): mixed { return $this->i; }
             function next(): void { $this->i++; } function rewind(): void { $this->i = 0; } function valid(): bool { return $this->i < 3; } }
         function g($agg) { yield from $agg; }
         $agg = new Agg; $it = new It; $s = 0;
         for ($i = 0; $i < $n; $i++) {
             foreach ($agg as $v) { $s += $v; }
             foreach ($it as $k => $v) { $s += $k; }
             foreach (g($agg) as $v) { $s += $v; }
         }
         echo $s / $n;",
        Count::NameLookup,
    );
    assert_eq!(out, "9");
}

/// A ReflectionMethod (and a parameter it made) holds the method it was
/// made for, as PHP's reflection_object holds the zend_function, and its
/// hidden properties are found in its few slots without hashing their
/// names: asking it again finds no member by name.
#[test]
fn reflection_holds_its_method() {
    let out = flat(
        "class P { function m(int $a, $b = 2) {} }
         class C extends P {}
         $rm = new ReflectionMethod('C', 'm'); $rp = $rm->getParameters()[1];
         $s = 0;
         for ($i = 0; $i < $n; $i++) {
             $s += ($rm->isPublic() ? 1 : 0) + $rm->getNumberOfParameters() + strlen($rm->getDeclaringClass()->getName())
                 + $rp->getPosition() + strlen($rm->getName()) + strlen($rm->class);
         }
         echo $s / $n, ' ', $rm->class, ' ', $rp->getName(), ' ', $rp->getDeclaringFunction()->getName();",
        Count::MemberLookup,
    );
    assert_eq!(out, "7 P b m");
}

/// A property named at run time (`$o->$name`) on a class with few
/// properties is found by going through them, visibility as before:
/// no name hashed.
#[test]
fn few_properties_are_found_without_hashing() {
    let out = flat(
        "class B { private $c = 30; function getB($p) { return $this->$p; } }
         class D extends B { public $a = 1; protected $b = 2; private $c = 3; function get($p) { return $this->$p; } }
         $d = new D; $s = 0;
         for ($i = 0; $i < $n; $i++) { foreach (['a', 'b', 'c'] as $p) { $s += $d->get($p); } $s += $d->getB('c'); $p = 'a'; $s += $d->$p; }
         echo $s / $n, ' ', isset($d->c) ? 'y' : 'n';",
        Count::MemberLookup,
    );
    assert_eq!(out, "37 n");
}

/// Writing a static property (`C::$x = …`, `C::$x .= …`, through `self`
/// and `static` too) uses the instruction's cache entry for the class and
/// scope, as PHP's runtime cache slot of ASSIGN_STATIC_PROP does: the
/// property is found by name once, not per write.
#[test]
fn static_property_writes_use_the_cache() {
    let out = flat(
        "class S { public static $x = 0; protected static $y = 1;
             static function bump($i) { static::$y += $i; self::$x = $i; return static::$y; } }
         class T extends S {}
         $s = 0;
         for ($i = 0; $i < $n; $i++) { $s += T::bump(1); S::$x = $i; S::$x .= '!'; }
         echo S::$x, ' ', $s, ' ', (new ReflectionProperty('T', 'y'))->getValue();",
        Count::MemberLookup,
    );
    assert_eq!(out, "19! 230 21");
}

/// The same from an instance method on a subclass instance, where the
/// called class (`static`) is `$this`'s class.
#[test]
fn static_property_writes_from_an_instance_method_use_the_cache() {
    let out = flat(
        "class S { public static $x = 0; protected static $y = 1;
             function bump($i) { static::$y += $i; static::$x = $i; static::$x .= '!'; return static::$y; } }
         class T extends S {}
         $t = new T; $s = 0;
         for ($i = 0; $i < $n; $i++) { $s += $t->bump(1); }
         echo S::$x, ' ', $s, ' ', (new ReflectionProperty('T', 'y'))->getValue();",
        Count::MemberLookup,
    );
    assert_eq!(out, "1! 230 21");
}

/// `$a[$k] = $v` where `$a` is a by-reference parameter, an int key and a
/// string key into a hashed array: one lookup of the key per write, as
/// ZEND_ASSIGN_DIM's (no second lookup to ask whether the element is bound
/// to `$a`'s own cell, `$a[$k] = &$a`: the write finds that out from the
/// element it has in hand). The 100 more writes at n = 120 than at n = 20
/// are 100 more lookups.
#[test]
fn a_write_through_a_reference_looks_up_its_key_once() {
    for (init, key) in [("['x' => 0, 3 => 0, 5 => 0]", "$i & 1 ? 5 : 3"), ("['x' => 0, 'p' => 0, 'q' => 0]", "$i & 1 ? 'q' : 'p'")] {
        let body = format!(
            "function f(array &$a, $k, $v) {{ $a[$k] = $v; }}
             $a = {init};
             for ($i = 0; $i < $n; $i++) {{ f($a, {key}, $i); }}
             echo implode(',', $a);"
        );
        run(&body, 1, Count::Find);
        let (a, b) = (run(&body, 20, Count::Find), run(&body, 120, Count::Find));
        assert_eq!(a.0, "0,18,19", "{key}");
        assert_eq!(b.1 - a.1, 100, "{key}: {} lookups at n = 20, {} at n = 120", a.1, b.1);
    }
}
