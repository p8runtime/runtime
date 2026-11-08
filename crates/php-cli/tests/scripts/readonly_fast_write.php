<?php
// A readonly or set-visibility property written through the property cache
// (the second run of each site is the cached one): its first write from the
// declaring class, and every case that must still fail or go elsewhere.
class P {
    public function __construct(public readonly int $a, public readonly ?P $next = null) {}
}
class Q {
    public readonly float $f;
    public private(set) int $n = 0;
    public protected(set) string $s = '';
    public function __construct(int $f) { $this->f = $f; }
    public function again() { $this->f = 2.5; }
    public function bump() { $this->n++; $this->n = $this->n + 10; $this->s = "s{$this->n}"; }
}
class R extends Q {
    public function child() { $this->s = 'child'; }
    public function childN() { $this->n = 99; }
}
class M {
    public readonly int $x;
    public function __set($k, $v) { echo "__set $k\n"; }
    public function __construct() { $this->x = 5; }
}
class C {
    public function __construct(public readonly array $items) {}
    public function __clone() { $this->items = [...$this->items, 'cloned']; }
}
class S {
    public readonly int $v;
    public function set($v) { $this->v = $v; }
}
for ($i = 0; $i < 2; $i++) {
    echo "-- run $i\n";
    $p = new P($i, new P(10 + $i));
    var_dump($p->a, $p->next->a);
    try { $p->a = 3; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    $q = new Q(3);
    var_dump($q->f);
    try { $q->again(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    $q->bump();
    var_dump($q->n, $q->s);
    try { $q->n = 4; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    try { $q->s = 'x'; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    $r = new R(7);
    $r->child();
    var_dump($r->s);
    try { $r->childN(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    $m = new M();
    try { var_dump($m->x); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    $c = new C([1]);
    $d = clone $c;
    var_dump($c->items, $d->items);
    $s = new S();
    try { $s->set("12"); var_dump($s->v); $s->set(13); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    $s2 = new S();
    try { $s2->set("x"); } catch (TypeError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    try { $s2->v = 1; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    // One site, run from different scopes (the same code bound to each):
    // the cache may answer only where the declaring class writes.
    $setN = function ($o, $v) { $o->n = $v; };
    $initV = function ($o, $v) { $o->v = $v; };
    foreach ([Q::class, null, R::class, Q::class] as $scope) {
        $f = Closure::bind($setN, null, $scope);
        $q2 = new Q(1);
        try { $f($q2, 5); echo "n=", $q2->n, "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    }
    foreach ([S::class, null, S::class, P::class] as $scope) {
        $f = Closure::bind($initV, null, $scope);
        $s3 = new S();
        try { $f($s3, 7); echo "v=", $s3->v, "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    }
}
