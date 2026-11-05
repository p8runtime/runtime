<?php
// clone() as a function and clone-with (PHP 8.5): the construct and
// the function, updated properties as the caller assigns them, readonly
// properties writable once more in __clone and in the updates.

// (Without object ids: which ids are free again is not this case's business.)
function show($label, $v) { ob_start(); var_dump($v); $out = ob_get_clean(); echo $label, ": ", preg_replace('/\)#\d+ /', ') ', $out); }
function attempt($label, $f) {
    try { $f(); } catch (Throwable $e) { echo $label, ": ", $e::class, ": ", $e->getMessage(), "\n"; }
}

class Point {
    public function __construct(public int $x = 0, public int $y = 0) {}
    public function withX(int $x) { return clone($this, ['x' => $x]); }
}

$p = new Point(1, 2);
show("construct", clone $p);
show("parens", clone($p));
show("trailing comma", clone($p,));
show("with", clone($p, ['y' => 5]));
show("with method", $p->withX(9));
show("original", $p);
show("int keys", clone(new stdClass, ['a', 'b', 'k' => 'v']));
show("empty with", clone($p, []));
$with = ['x' => 7];
show("with from a variable", clone($p, $with));
show("named", clone(object: $p, withProperties: ['y' => 3]));
show("spread", clone(...[$p, ['x' => 4]]));
$f = clone(...);
show("first-class callable", $f($p, ['x' => 8]));
show("string callable", array_map('clone', [$p])[0] !== $p);
show("call_user_func", call_user_func('clone', $p, ['y' => 11]));
attempt('clone($p)->x clones ($p)->x', fn() => clone($p)->x + 1);
show('(clone($p))->x', (clone($p))->x + 1);

// Evaluation order: the object, then the array.
function o() { echo "o "; return new Point; }
function a() { echo "a "; return ['x' => 1]; }
show("order", clone(o(), a()));

// Visibility: the caller's scope.
class P {
    public $a = 'a';
    protected $b = 'b';
    private $c = 'c';
    public private(set) string $d = 'd';
    public function m() { return clone($this, ['b' => 'B', 'c' => 'C', 'd' => 'D']); }
}
class C extends P {
    public function m2() { return clone($this, ['b' => 'B2']); }
    public function m3() { return clone($this, ['d' => 'no']); }
}
show("own scope", (new P)->m());
show("child scope", (new C)->m2());
attempt("private(set) from child", fn() => (new C)->m3());
attempt("protected from outside", fn() => clone(new P, ['b' => 'no']));
attempt("private(set) from outside", fn() => clone(new P, ['d' => 'no']));

// Hooks and __set run.
class Hooked {
    public string $name = '' { set { echo "set($value) "; $this->name = strtoupper($value); } }
}
show("hook", clone(new Hooked, ['name' => 'abc']));
class Magic {
    private array $data = [];
    public function __set($n, $v) { echo "__set($n) "; $this->data[$n] = $v; }
}
show("__set", clone(new Magic, ['p' => 1, 'q' => 2]));
class Fails {
    public string $a = '' { set { if ($value === 'bad') throw new Exception("bad a"); $this->a = $value; } }
    public string $b = '';
}
attempt("hook throws", fn() => clone(new Fails, ['b' => 'set', 'a' => 'bad']));
class Typed { public int $n = 0; }
attempt("type", fn() => clone(new Typed, ['n' => 'x']));
show("coerced", clone(new Typed, ['n' => '42']));

// __clone runs first, then the updates.
class WithClone {
    public $log = [];
    public $v = 'orig';
    public function __clone() { $this->log[] = "__clone saw v={$this->v}"; $this->v = 'from __clone'; }
}
show("__clone then with", clone(new WithClone, ['v' => 'from with']));
class PrivateClone { private function __clone() {} }
attempt("private __clone", fn() => clone(new PrivateClone, ['a' => 1]));
attempt("private __clone, no with", fn() => clone(new PrivateClone));

// readonly: once more in __clone, and once more in the updates.
final class Money {
    public function __construct(public readonly int $amount, public readonly string $cur) {}
    public function __clone() { $this->amount++; }
    public function in(string $cur) { return clone($this, ['cur' => $cur, 'amount' => 1]); }
}
$m = new Money(10, 'CAD');
show("readonly __clone", clone $m);
show("readonly __clone and with", $m->in('USD'));
attempt("readonly from outside scope", fn() => clone($m, ['cur' => 'USD']));
attempt("dynamic on a readonly's class", fn() => clone($m, ['nope' => 1]));
readonly class Pair {
    public function __construct(public public(set) int $a, public public(set) int $b) {}
}
$pair = clone(new Pair(1, 2), ['a' => 3]);
show("readonly class with", $pair);
attempt("readonly after clone-with", function () use ($pair) { $pair->b = 4; });
class Twice {
    public function __construct(public readonly int $v) {}
    public function __clone() { $this->v = 2; $this->v = 3; }
}
attempt("readonly twice in __clone", fn() => clone new Twice(1));
class TypeFirst {
    public function __construct(public readonly int $v) {}
    public function __clone() {
        try { $this->v = "no"; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
        $this->v = 5;
    }
}
show("type error keeps the write", clone new TypeFirst(1));
class UnsetClone {
    public function __construct(public readonly stdClass $o) {}
    public function __clone() { unset($this->o); $this->o = new stdClass; $this->o->fresh = true; }
}
show("unset in __clone", clone new UnsetClone(new stdClass));
class Indirect {
    public function __construct(public readonly array $list) {}
    public function __clone() { $this->list[] = 1; }
}
attempt("indirect in __clone", fn() => clone new Indirect([]));
class HookSetsReadonly {
    public string $foo { set { $this->foo = $value; $this->bar = 'by hook'; } }
    public public(set) readonly string $bar;
}
show("hook writes readonly", clone(new HookSetsReadonly, ['foo' => 'f']));
attempt("hook used it up", fn() => clone(new HookSetsReadonly, ['foo' => 'f', 'bar' => 'again']));
$ro = new Money(1, 'CAD');
attempt("readonly outside clone", function () use ($ro) { $ro->amount = 2; });

// References and names.
$val = 'r';
$refs = ['x' => &$val];
attempt("reference", fn() => clone(new stdClass, $refs));
unset($val);
show("reference only the array holds", clone(new stdClass, $refs));
class Mangled { private string $bar = 'b'; }
attempt("mangled name", fn() => clone(new Mangled, ["\0Mangled\0bar" => 'x']));
attempt("dynamic property", fn() => clone(new Point, ['z' => 1]));

// Argument errors.
attempt("not an object", fn() => clone(1, []));
attempt("not an array", fn() => clone(new stdClass, 1));
attempt("no arguments", fn() => clone());
attempt("too many", fn() => clone(new stdClass, [], 1));
attempt("uncloneable", fn() => clone((function () { yield 1; })(), ['a' => 1]));
var_dump(function_exists('clone'));
$r = new ReflectionFunction('clone');
echo $r->getName(), " ", $r->getNumberOfParameters(), " ", $r->getNumberOfRequiredParameters(), "\n";
