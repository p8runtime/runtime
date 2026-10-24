<?php
class C {
    public int $a = 1;
    public $b;
    public function __construct() { echo "ctor\n"; $this->a = 5; }
    public function __destruct() { echo "dtor {$this->a}\n"; }
}
$r = new ReflectionClass(C::class);
$g = $r->newLazyGhost(function (C $o) { echo "init ghost\n"; $o->__construct(); });
var_dump($r->isUninitializedLazyObject($g));
var_dump($g);
echo $g->a, "\n";
var_dump($r->isUninitializedLazyObject($g), $g);

$p = $r->newLazyProxy(function (C $o) { echo "factory\n"; return new C; });
$p->b = 'x';
var_dump($p, $p->b, $r->isUninitializedLazyObject($p));

$h = $r->newLazyGhost(function ($o) { echo "never\n"; });
$r->getProperty('a')->skipLazyInitialization($h);
$r->getProperty('b')->setRawValueWithoutLazyInitialization($h, 'raw');
var_dump($r->isUninitializedLazyObject($h), $h->a, $h->b);

$e = $r->newLazyGhost(function ($o) { $o->a = 9; throw new Exception('boom'); });
try { $e->a; } catch (Exception $x) { echo $x->getMessage(), "\n"; }
var_dump($r->isUninitializedLazyObject($e));
$r->markLazyObjectAsInitialized($e);
var_dump($e->a, $r->getLazyInitializer($e));

$k = $r->newLazyGhost(function ($o) { echo "init k\n"; });
foreach ($k as $name => $v) { echo "$name\n"; }
$c = clone $p;
var_dump($c->b);
try { $r->newLazyGhost(fn() => null, 99); } catch (ReflectionException $x) { echo $x->getMessage(), "\n"; }
#[AllowDynamicProperties] class D {}
$d = new D; $d->dyn = 1; echo "no deprecation\n";
echo "end\n";
