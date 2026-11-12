<?php
// One method call site, run again and again: what it remembers about the
// method it found must be asked again when the receiver's class or the
// calling scope is another, and what it calls must run as a method of the
// class that declares it.

// 1. The receiver's class changes between executions of one site, and the
// two classes take the argument differently (by value, by reference).
class ByVal { function m($x) { $x .= '!'; return 'ByVal'; } }
class ByRef { function m(&$x) { $x .= '!'; return 'ByRef'; } }
class Both extends ByRef { }
$v = 'v';
foreach ([new ByVal, new ByVal, new ByRef, new ByRef, new ByVal, new Both, new Both, new ByVal, new ByRef] as $o) {
    echo $o->m($v), ' ', $v, "\n";
}

// 2. An inherited method runs in the class that declares it: its private
// members, self:: and static:: are that class's, two and three parents up.
class Base {
    private $secret = 'base secret';
    private function hidden($x) { return "Base::hidden($x)"; }
    static function who() { return 'Base'; }
    function show($x) { return $this->hidden($x) . ' ' . $this->secret . ' ' . self::who() . ' ' . static::who() . ' ' . static::class; }
}
class Mid extends Base {
    private $secret = 'mid secret';
    private function hidden($x) { return "Mid::hidden($x)"; }
    static function who() { return 'Mid'; }
}
class Leaf extends Mid {
    static function who() { return 'Leaf'; }
    function own($x) { return 'Leaf::own ' . parent::show($x); }
}
class Leafer extends Leaf { }
foreach ([new Base, new Mid, new Leaf, new Leafer, new Leafer, new Leaf, new Mid, new Base, new Base] as $i => $o) {
    echo $o->show($i), "\n";
}
foreach ([new Leaf, new Leafer, new Leafer, new Leaf] as $i => $o) {
    echo $o->own($i), "\n";
}

// 3. The same site from two scopes: a closure bound to a class may call
// its private method, the same closure unbound may not.
class Vault {
    private function open($k) { return "opened with $k"; }
    function opener() { return function ($o, $k) { return $o->open($k); }; }
}
$vault = new Vault;
$inside = $vault->opener();
$outside = Closure::bind($inside, null, null);
foreach ([$inside, $inside, $outside, $inside, $outside, $outside, $inside] as $i => $f) {
    try {
        echo $f($vault, $i), "\n";
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}

// 4. A private method and a public one of the same name: the caller's
// class decides which runs, site by site.
class Owner {
    private function pick($x) { return "Owner::pick($x)"; }
    function viaOwner($o, $x) { return $o->pick($x); }
}
class Heir extends Owner {
    public function pick($x) { return "Heir::pick($x)"; }
    function viaHeir($o, $x) { return $o->pick($x); }
}
$h = new Heir;
$own = new Owner;
for ($i = 0; $i < 3; $i++) {
    echo $h->viaOwner($h, $i), ' ', $h->viaHeir($h, $i), ' ', $h->pick($i), ' ', $h->viaOwner($own, $i), "\n";
}

// 5. A trait's method is the using class's; an enum's method; a static
// method called on an object; a generator method; __call.
trait Greets { private $greeting = 'hello'; function greet($n) { return $this->greeting . ' ' . $n . ' from ' . self::class . '/' . static::class; } }
class English { use Greets; }
class Loud extends English { }
enum Suit { case Hearts; case Spades; function label($p) { return $p . $this->name; } }
class Tools {
    static function twice($x) { return 2 * $x; }
    function gen($n) { for ($i = 0; $i < $n; $i++) { yield $i; } }
    function __call($name, $args) { return $name . '(' . implode(',', $args) . ')'; }
}
$t = new Tools;
foreach ([new English, new Loud, new Loud, new English] as $i => $o) {
    echo $o->greet($i), "\n";
}
foreach ([Suit::Hearts, Suit::Spades, Suit::Spades] as $s) {
    echo $s->label('suit '), "\n";
}
for ($i = 0; $i < 3; $i++) {
    echo $t->twice($i), ' ', implode(',', iterator_to_array($t->gen($i))), ' ', $t->missing($i, $i), "\n";
}

// 6. Too few arguments at a site that has run before.
class NeedsOne { function take($a, $b = 2, ...$rest) { return "one: $a $b " . count($rest); } }
class NeedsTwo extends NeedsOne { function take($a, $b = null, ...$rest) { if ($b === null) { throw new LogicException('no b'); } return "two: $a $b"; } }
class NeedsThree { function take($a, $b, $c) { return "three: $a $b $c"; } }
foreach ([new NeedsOne, new NeedsOne, new NeedsThree, new NeedsOne, new NeedsTwo, new NeedsThree, new NeedsOne] as $i => $o) {
    try {
        echo $o->take($i), "\n";
        echo $o->take($i, $i, $i, $i), "\n";
    } catch (Throwable $e) {
        echo get_class($e), ': ', strtok($e->getMessage(), ','), "\n";
    }
}

// 7. Recursion through one site, and a receiver that is a temporary or a
// reference.
class Fact {
    public $calls = 0;
    function of($n) { $this->calls++; return $n <= 1 ? 1 : $n * $this->of($n - 1); }
    function __destruct() { echo "Fact gone after {$this->calls}\n"; }
}
for ($i = 3; $i < 6; $i++) {
    echo (new Fact)->of($i), "\n";
}
function through_global() { global $g; $r = 0; for ($i = 0; $i < 3; $i++) { $r += $g->of(3); } return $r; }
$g = new Fact;
echo through_global(), ' ', $g->calls, "\n";
$g = null;
echo "done\n";
