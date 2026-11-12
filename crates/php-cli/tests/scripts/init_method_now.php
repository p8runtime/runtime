<?php
// A method call none of whose arguments runs anything (no argument, or
// literals only) reads its receiver, finds the method and calls it in one
// step. What a call site learnt for one class, or from one scope, must not
// answer for another; and everything a call can meet (a temporary receiver,
// null, __call, a static method, a generator, too few arguments, a builtin
// method) behaves the second time through a site as the first.
class A {
    public $n = 0;
    function hit() { return static::class . " in A::hit " . ++$this->n; }
    function lit($a, $b = "dflt") { return "A::lit(" . var_export($a, true) . ", " . var_export($b, true) . ")"; }
    private function own() { return "A::own"; }
    function callOwn($o) { return $o->own(); }
    function viaThis() { return $this->hit(); }
    function me() { return $this; }
    static function st() { return "A::st, called " . static::class; }
    static function noThis() { return $this->hit(); }
    function gen() { yield "one"; yield "two"; }
    function need($a, $b) { return "need ran"; }
    function __destruct() { echo "[", static::class, " gone]\n"; }
}
class B extends A {
    function hit() { return "B::hit " . ++$this->n; }
    public function own() { return "B::own"; }
}
class M {
    function __call($name, $args) { return "__call $name(" . implode(",", $args) . ")"; }
}

// One site, two classes in turn.
function site($o) { return $o->hit(); }
$a = new A; $b = new B;
echo site($a), " | ", site($a), " | ", site($b), " | ", site($a), " | ", site($b), "\n";

// One site, one class, two scopes: A's own private method from inside A,
// then from outside.
echo $a->callOwn($a), ", ", $a->callOwn($a), ", ", $a->callOwn($b), ", ", $a->callOwn($a), "\n";
$f = function ($o) { return $o->own(); };
$inA = Closure::bind($f, null, A::class);
foreach ([$inA, $inA, $f, $inA, $f] as $n => $g) {
    try { echo $n, ": ", $g($a), "\n"; } catch (Error $e) { echo $n, ": ", $e->getMessage(), "\n"; }
}

// Literal arguments, each site twice.
for ($i = 0; $i < 2; $i++) {
    echo $a->lit(1), " ", $a->lit("x", 2.5), " ", $a->lit(null, true), "\n";
}

// `$this` as the receiver, in A and in a subclass; a chain.
echo $a->viaThis(), " | ", $a->viaThis(), " | ", $b->viaThis(), " | ", $a->me()->me()->hit(), "\n";
for ($i = 0; $i < 2; $i++) {
    try { echo A::noThis(), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
}

// A temporary receiver: released when the call is over.
for ($i = 0; $i < 2; $i++) {
    echo "temporary: ", (new A)->hit(), "\n";
    echo "temporary, literal: ", (new B)->lit(7), "\n";
}

// Not an object, after an object at the same site.
function onAny($n) { return $n->lit(1); }
foreach ([$a, $a, null, 5, "str", $a] as $v) {
    try { echo onAny($v), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
}
try { $undefined->hit(); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// No such method, and a class that takes any.
function any($o) { return $o->anything(1, "two"); }
foreach ([new M, new M, $a, new M] as $o) {
    try { echo any($o), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
}

// A static method on an object, a generator, too few arguments.
function kinds($o) {
    $out = $o->st() . ";";
    foreach ($o->gen() as $v) { $out .= " $v"; }
    try { $out .= "; " . $o->need(1); } catch (ArgumentCountError $e) { $out .= "; " . get_class($e) . ": " . $e->getMessage(); }
    return $out;
}
echo kinds($a), "\n", kinds($a), "\n", kinds($b), "\n";

// An object through a reference.
function throughGlobal() { global $a; return $a->hit(); }
echo throughGlobal(), " | ", throughGlobal(), "\n";

// Builtin methods and a closure's __invoke.
$list = new ArrayObject([1, 2, 3]);
$c = function ($x = "none") { return "closure got $x"; };
for ($i = 0; $i < 2; $i++) {
    echo $list->count(), ", ", $c->__invoke(), ", ", $c->__invoke(5), "\n";
}
echo "done\n";
