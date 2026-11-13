//! Copy-on-write: an array is copied only where PHP copies it (a write to
//! an array something else also holds). Each test runs a loop once with 1
//! and once with 101 iterations: the copies it makes must not grow with the
//! iterations, and the output must be PHP's (#399).

#[derive(Default)]
struct Capture(Vec<u8>);

impl p8::Host for Capture {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

/// `body` with `$n` set to `n`, as one request: its output and the arrays
/// it copied.
fn run(body: &str, n: u32) -> (String, u64) {
    let platform = p8::Platform::new(&[p8::extensions::STANDARD]);
    let mut isolate = platform.new_isolate();
    let script = platform.compile(format!("<?php $n = {n};\n{body}").as_bytes(), "t.php").expect("compiles");
    let mut host = Capture::default();
    let before = p8_runtime::array::copies();
    let status = isolate.new_context(&mut host).run(&script);
    let copies = p8_runtime::array::copies() - before;
    assert_eq!(status, 0, "{}", String::from_utf8_lossy(&host.0));
    (String::from_utf8(host.0).expect("utf-8"), copies)
}

/// The copies `body` makes do not depend on `$n`; its outputs at n = 1 and
/// n = 101.
fn no_copies_per_iteration(body: &str) -> (String, String) {
    let (one, at_one) = run(body, 1);
    let (many, at_many) = run(body, 101);
    assert_eq!(at_many, at_one, "copies grew with the iterations: {at_one} at n = 1, {at_many} at n = 101");
    (one, many)
}

/// ZEND_RECV_VARIADIC checks a typed variadic list in place: no copy per
/// call, and a coerced argument is still written into it.
#[test]
fn a_typed_variadic_is_checked_in_place() {
    let (one, many) = no_copies_per_iteration(
        "function f(int ...$a) { return array_sum($a); }
         function g(float ...$a) { return $a; }
         $s = 0; for ($i = 0; $i < $n; $i++) { $s += f($i, 1, 2); }
         echo $s, ' '; var_dump(g(1, 2));",
    );
    assert_eq!(one, "3 array(2) {\n  [0]=>\n  float(1)\n  [1]=>\n  float(2)\n}\n");
    assert!(many.starts_with("5353 array(2)"), "{many}");
}

/// ArrayIterator and ArrayObject read their storage in place
/// (spl_array_get_hash_table), and change it in place when nothing else
/// holds it: iterating is O(n), not O(n) per step.
#[test]
fn array_iterator_reads_its_storage_in_place() {
    let (one, many) = no_copies_per_iteration(
        "$o = new ArrayObject();
         for ($i = 0; $i < $n; $i++) { $o[] = $i; }
         $s = 0; foreach (new ArrayIterator($o->getArrayCopy()) as $k => $v) { $s += $k + $v; }
         $it = $o->getIterator(); for ($it->rewind(); $it->valid(); $it->next()) { $s += $it->current(); }
         echo $s, ' ', count($o), ' ', isset($o[0]) ? 'set' : 'unset';",
    );
    assert_eq!(one, "0 1 set");
    assert_eq!(many, "15150 101 set");
}

/// A write to ArrayObject storage the caller still holds separates it, as
/// PHP does: the caller's array is unchanged.
#[test]
fn array_object_storage_is_still_copy_on_write() {
    let (out, _) = run("$a = [1, 2]; $o = new ArrayObject($a); $o[] = 3; echo count($a), count($o);", 1);
    assert_eq!(out, "23");
}

/// SplObjectStorage looks up and attaches without copying its map.
#[test]
fn object_storage_changes_its_map_in_place() {
    let (one, many) = no_copies_per_iteration(
        "$s = new SplObjectStorage(); $kept = [];
         for ($i = 0; $i < $n; $i++) { $o = new stdClass; $kept[] = $o; $s[$o] = $i; $s[$o] = $i * 2; }
         $hits = 0; foreach ($kept as $o) { $hits += isset($s[$o]) ? $s[$o] : 0; }
         $s->addAll($s); $s->removeAll($s);
         echo $hits, ' ', count($s), ' ', isset($s[$kept[0]]) ? 'y' : 'n';",
    );
    assert_eq!(one, "0 0 n");
    assert_eq!(many, "10100 0 n");
}

/// debug_backtrace builds each frame once: no frame array is copied.
#[test]
fn debug_backtrace_copies_no_frame() {
    let (one, many) = no_copies_per_iteration(
        "function f() { return g(); }
         function g() { $t = debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS); return count($t) . isset($t[0]['args']) . $t[1]['function']; }
         $r = ''; for ($i = 0; $i < $n; $i++) { $r = f(1, 2); }
         echo $r, ' ', count(debug_backtrace());",
    );
    assert_eq!(one, "2f 0");
    assert_eq!(many, one);
}

/// A quiet read of a property's array (`isset($o->a[$k])`, `$o->a[$k] ??`,
/// `!empty(...)`, `$o->a ?? []`) lets go of its copy right after the read:
/// the write to that array in the next statement changes it in place, on
/// `$this` and on another object alike.
#[test]
fn a_quiet_property_read_keeps_no_copy_for_the_next_write() {
    let (one, many) = no_copies_per_iteration(
        "class C {
             public $items = ['x' => 0];
             function a($k) { if (isset($this->items[$k])) { $x = 1; } $this->items[$k] = 1; }
             function b($k) { $v = $this->items[$k] ?? 0; $this->items[$k] = $v + 1; }
             function c($k) { if (!empty($this->items[$k])) { $x = 1; } $this->items[$k] = 2; }
             function d($k) { $v = $this->items ?? []; unset($v); $this->items[$k] = 3; }
         }
         function e($o, $k) { if (isset($o->items[$k])) { $x = 1; } $o->items[$k] = 4; }
         function f($o, $k) { $v = $o->items[$k] ?? 0; $o->items[$k] = $v + 5; }
         $c = new C();
         for ($i = 0; $i < $n; $i++) { $k = 'k' . ($i % 3); $c->a($k); $c->b($k); $c->c($k); $c->d($k); e($c, $k); f($c, $k); }
         ksort($c->items); echo json_encode($c->items);",
    );
    assert_eq!(one, r#"{"k0":9,"x":0}"#);
    assert_eq!(many, r#"{"k0":9,"k1":9,"k2":9,"x":0}"#);
}
