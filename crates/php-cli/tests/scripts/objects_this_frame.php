<?php
// `$this->p` in an instance method reads the frame's $this: reads,
// writes, compound writes and increments; in constructors (promotion),
// traits, hooks, closures in methods, generators, and called in ways
// where the method has no $this.

trait Tally {
    private int $tally = 0;
    public function bump(): int { $this->tally++; $this->tally += 2; return $this->tally; }
}
class Box {
    use Tally;
    public array $items = [];
    public ?Box $next = null;
    public function __construct(public string $name, protected int $size = 0) {}
    public function add($v) { $this->items[] = $v; $this->size = $this->size + 1; return $this; }
    public function size() { return $this->size; }
    public function chain() { $this->next = new Box($this->name . '+'); return $this->next->name; }
    public function viaClosure() { return (function () { return $this->name . '/' . $this->size; })(); }
    public function viaArrow() { return (fn() => $this->size * 10)(); }
    public function gen() { for ($i = 0; $i < 3; $i++) { $this->size++; yield $this->size; } }
    public function missing() { return $this->nope; }
    public function readUnset() { unset($this->items); return isset($this->items); }
    public function incStr() { $this->size--; --$this->size; return $this->size; }
}
$b = new Box('a');
$b->add(1)->add(2);
var_dump($b->size(), $b->items, $b->bump(), $b->bump(), $b->chain(), $b->viaClosure(), $b->viaArrow());
foreach ($b->gen() as $v) echo $v, " ";
echo "\n";
var_dump(@$b->missing());
var_dump($b->readUnset());
var_dump($b->incStr());

class Hooked {
    public int $x = 1 { set { $this->x = $value * 2; } get { return $this->x + 1; } }
    public function twice() { $this->x = 5; return $this->x; }
}
var_dump((new Hooked)->twice());

class Magic {
    private array $d = [];
    public function __get($n) { echo "get $n\n"; return $this->d[$n] ?? null; }
    public function __set($n, $v) { echo "set $n\n"; $this->d[$n] = $v; }
    public function run() { $this->a = 1; $this->a += 5; return $this->a; }
}
var_dump((new Magic)->run());

class Ro {
    public function __construct(public readonly int $v) {}
    public function change() { $this->v = 2; }
    public function inc() { $this->v++; }
}
$r = new Ro(1);
try { $r->change(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $r->inc(); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// No $this: a static call of an instance method, an unbound closure.
class NoThis { public $p = 1; public function get() { return $this->p; } }
try { NoThis::get(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$m = (new ReflectionMethod(NoThis::class, 'get'))->getClosure(new NoThis);
var_dump($m());
$unbound = Closure::bind(function () { return isset($this) ? $this->p : 'no this'; }, null, NoThis::class);
var_dump($unbound());
$bound = Closure::bind(function () { return $this->p; }, new NoThis, NoThis::class);
var_dump($bound());

// Subclass calling the parent's method that writes through $this.
class Base { protected $log = []; public function note($s) { $this->log[] = $s; return count($this->log); } }
class Child extends Base { public function both() { $this->note('a'); return parent::note('b') + $this->note('c'); } }
var_dump((new Child)->both());
