<?php
// Typed property writes through the property cache (every site runs more
// than once, so its entry answers): each type against every kind of value,
// what the type takes as it is, what weak mode converts and the TypeError
// for the rest; `$o->p = v` and `$this->p = v`; one site seeing classes
// whose property of that name has another type; readonly and
// set-visibility properties written from their class, from a subclass,
// from outside and from one site run in several scopes; a clone's
// re-initialization; a class with __set and an unset typed slot; and a
// typed property's slot holding a reference.
interface Shape {}
class Base implements Shape {}
class Sub extends Base {}
class Other {}
enum Suit { case Hearts; }

class T {
    public int $p_int = 0;
    public float $p_float = 0.0;
    public string $p_string = '';
    public bool $p_bool = false;
    public array $p_array = [];
    public ?int $p_nint = null;
    public int|string $p_union = 0;
    public float|string $p_fs = 0.0;
    public mixed $p_mixed = null;
    public iterable $p_iter = [];
    public object $p_object;
    public true $p_true = true;
    public false $p_false = false;
    public ?Base $p_base = null;
    public Shape $p_shape;
    public int|float $p_num = 0;
    public $p_untyped;
    public function setInt($v) { $this->p_int = $v; return $this->p_int; }
    public function setNFloat($v) { $this->p_float = $v; return $this->p_float; }
    // (The write's result used: a value the type takes as it is, or none.)
    public function usedNInt($v) { $r = ($this->p_nint = $v); return [$r, $this->p_nint]; }
}

function show(mixed $v): string {
    return get_debug_type($v) . (is_scalar($v) || $v === null ? ' ' . var_export($v, true) : (is_array($v) ? ' ' . json_encode($v) : ''));
}

function w_int($o, $v) { $o->p_int = $v; return $o->p_int; }
function w_float($o, $v) { $o->p_float = $v; return $o->p_float; }
function w_string($o, $v) { $o->p_string = $v; return $o->p_string; }
function w_bool($o, $v) { $o->p_bool = $v; return $o->p_bool; }
function w_array($o, $v) { $o->p_array = $v; return $o->p_array; }
function w_nint($o, $v) { $o->p_nint = $v; return $o->p_nint; }
function w_union($o, $v) { $o->p_union = $v; return $o->p_union; }
function w_fs($o, $v) { $o->p_fs = $v; return $o->p_fs; }
function w_mixed($o, $v) { $o->p_mixed = $v; return $o->p_mixed; }
function w_iter($o, $v) { $o->p_iter = $v; return $o->p_iter; }
function w_object($o, $v) { $o->p_object = $v; return $o->p_object; }
function w_true($o, $v) { $o->p_true = $v; return $o->p_true; }
function w_false($o, $v) { $o->p_false = $v; return $o->p_false; }
function w_base($o, $v) { $o->p_base = $v; return $o->p_base; }
function w_shape($o, $v) { $o->p_shape = $v; return $o->p_shape; }
function w_num($o, $v) { $o->p_num = $v; return $o->p_num; }
function w_untyped($o, $v) { $o->p_untyped = $v; return $o->p_untyped; }
function w_this_int($o, $v) { return $o->setInt($v); }
function w_this_float($o, $v) { return $o->setNFloat($v); }

$values = [
    7, -3, 1.5, 2.0, "12", "12.5", " 12", "abc", "1e3", str_repeat("9", 2), "",
    true, false, null, [1, 2], [], new Base, new Sub, new Other,
    function () {}, Suit::Hearts, new ArrayIterator([1]), new T,
];
$writers = ['w_int', 'w_float', 'w_string', 'w_bool', 'w_array', 'w_nint', 'w_union', 'w_fs', 'w_mixed', 'w_iter',
    'w_object', 'w_true', 'w_false', 'w_base', 'w_shape', 'w_num', 'w_untyped', 'w_this_int', 'w_this_float'];

$o = new T;
foreach ($writers as $w) {
    echo "== $w\n";
    foreach ($values as $v) {
        $line = [];
        for ($i = 0; $i < 2; $i++) {
            try {
                $line[] = show($w($o, $v));
            } catch (TypeError $e) {
                $line[] = 'TypeError: ' . $e->getMessage();
            }
        }
        echo show($v), ' -> ', $line[0] === $line[1] ? $line[0] : implode(' | ', $line), "\n";
    }
}

echo "== used result\n";
foreach ([3, null, -1, "x", [], new Base] as $v) {
    for ($i = 0; $i < 2; $i++) {
        try {
            echo show($v), ' -> ', implode(', ', array_map('show', $o->usedNInt($v))), "\n";
        } catch (TypeError $e) {
            echo show($v), ' -> TypeError: ', $e->getMessage(), "\n";
        }
    }
}

// One site, objects of classes whose property `v` has another type (or
// none), then each again: the entry is filled for each class anew.
class VI { public int $v = 0; }
class VS { public string $v = ''; }
class VU { public $v; }
class VF { public ?float $v = null; }
function wv($o, $x) { $o->v = $x; return $o->v; }
echo "== one site, several classes\n";
$objs = [new VI, new VS, new VU, new VF];
foreach ([5, "6", 1.25, "x", null, true] as $x) {
    for ($round = 0; $round < 2; $round++) {
        foreach ($objs as $obj) {
            try {
                $r = show(wv($obj, $x));
                echo get_class($obj), ' ', show($x), ' -> ', $r, "\n";
            } catch (TypeError $e) {
                echo get_class($obj), ' ', show($x), ' -> TypeError: ', $e->getMessage(), "\n";
            }
        }
    }
}

// Readonly and set visibility.
class RO {
    public function __construct(public readonly int $a, protected readonly ?RO $next = null, private readonly string $s = 'd') {}
    public function setA($v) { $this->a = $v; }
    public function s() { return $this->s; }
}
class ROC extends RO {
    public function __construct() { }
    public function initA($v) { $this->a = $v; }
}
class SV {
    public private(set) int $n = 0;
    public protected(set) float $f = 0.0;
    public function setN($v) { $this->n = $v; return $this->n; }
    public function setF($v) { $this->f = $v; return $this->f; }
}
class SVC extends SV {
    public function childF($v) { $this->f = $v; return $this->f; }
    public function childN($v) { $this->n = $v; return $this->n; }
}
class RL {
    public readonly int $x;
    public readonly array $list;
    public function __construct(int $x) { $this->x = $x; $this->list = [$x]; }
    public function __clone() { $this->list = [...$this->list, 'clone']; $this->x = $this->x + 100; }
}
function attempt(string $label, callable $f): void {
    try {
        $r = $f();
        echo $label, ': ', show($r), "\n";
    } catch (Throwable $e) {
        echo $label, ': ', get_class($e), ': ', $e->getMessage(), "\n";
    }
}
echo "== readonly, set visibility\n";
for ($i = 0; $i < 3; $i++) {
    $r = new RO($i, new RO(10 + $i), "s$i");
    attempt("ro a", fn () => $r->a);
    attempt("ro s", fn () => $r->s());
    attempt("ro again", fn () => $r->setA(9));
    attempt("ro outside", function () use ($r) { $r->a = 5; });
    $c = new ROC;
    attempt("ro child init", fn () => $c->initA("4$i"));
    attempt("ro child read", fn () => $c->a);
    attempt("ro child again", fn () => $c->initA(1));
    $c2 = new ROC;
    attempt("ro child bad", fn () => $c2->initA("x"));
    attempt("ro outside uninit", function () use ($c2) { $c2->a = 1; });
    $s = new SV;
    attempt("sv n", fn () => $s->setN("3$i"));
    attempt("sv n bad", fn () => $s->setN([]));
    attempt("sv f", fn () => $s->setF($i));
    attempt("sv outside n", function () use ($s) { $s->n = 1; });
    attempt("sv outside f", function () use ($s) { $s->f = 1.0; });
    $sc = new SVC;
    attempt("svc child f", fn () => $sc->childF(2.5));
    attempt("svc child n", fn () => $sc->childN(2));
    $l = new RL($i);
    $m = clone $l;
    attempt("clone", fn () => [$l->x, $m->x, $m->list]);
}

// One site run in several scopes (one closure bound to each class): only
// the declaring class's run may initialize, whichever ran there before.
class BA { public readonly int $v; public private(set) string $w; }
class BB { }
$set = function ($o, $v) { $o->v = $v; $o->w = "w$v"; return $o->v; };
echo "== one site, several scopes\n";
foreach ([BA::class, BB::class, BA::class, null, BA::class, BB::class] as $k => $scope) {
    $f = Closure::bind($set, null, $scope);
    attempt('scope ' . ($scope ?? 'none'), fn () => $f(new BA, $k));
}

// A class with __set: an unset typed slot is __set's; a set one is written.
class MG {
    public int $i = 1;
    public ?string $s;
    public function __set($k, $v) { echo "__set $k ", show($v), "\n"; }
    public function wi($v) { $this->i = $v; return $this->i; }
    public function ws($v) { $this->s = $v; }
}
echo "== magic\n";
for ($i = 0; $i < 2; $i++) {
    $m = new MG;
    attempt("mg i", fn () => $m->wi($i + 5));
    attempt("mg i again", fn () => $m->wi("1$i"));
    attempt("mg s", fn () => $m->ws("a$i"));
    attempt("mg s read", fn () => $m->s);
    attempt("mg i bad", fn () => $m->wi("z"));
}

// The slot holds a reference: the value goes through it, typed.
echo "== references\n";
for ($i = 0; $i < 2; $i++) {
    $t = new T;
    $ref = &$t->p_int;
    attempt("ref int", fn () => w_int($t, 40 + $i));
    attempt("ref var", fn () => $ref);
    attempt("ref bad", fn () => w_int($t, "q"));
    $f = new T;
    $both = &$f->p_float;
    attempt("ref float int", fn () => w_float($f, $i));
    attempt("ref float var", fn () => $both);
    unset($ref, $both);
}
