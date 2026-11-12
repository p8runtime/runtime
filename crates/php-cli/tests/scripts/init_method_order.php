<?php
// A method call takes its receiver and finds its method before its
// arguments run (#490): an argument that replaces the variable does not
// change what is called, and the errors of finding the method come before
// anything an argument does.
class A { function m($x, $y) { return "A::m"; } }
class B { function m(&$x, $y) { $x = "changed"; return "B::m"; } }
function swap(&$o) { $o = new B; return 1; }
function eff() { echo "eff "; return 1; }
class T {
    function m($x, $y) { return "T::m"; }
    private function hidden($x) { return "hidden"; }
    protected function guarded($x) { return "guarded"; }
}

// 1: the argument assigns the receiver's variable.
$o = new A; $v = "v";
echo $o->m($v, $o = new B), " v=$v ", get_class($o), "\n";
// 2: the argument replaces it through a reference.
$o = new A; $v = "v";
echo $o->m($v, swap($o)), " v=$v ", get_class($o), "\n";
// (The other way round: the method found takes the reference.)
$o = new B; $v = "v";
echo $o->m($v, $o = new A), " v=$v ", get_class($o), "\n";
// 3: no receiver: the error, and the argument never runs.
$n = null; $k = 0;
try { $n->m($k, $k = 5); } catch (Error $e) { echo $e->getMessage(), " k=$k\n"; }
// 4: no such method, or not visible from here: the same.
$t = new T;
try { $t->nope(eff()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $t->hidden(eff()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $t->guarded(eff()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
// 5: the error comes before an undefined argument's warning.
try { $n->m($undefined); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $t->nope($undefined); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $t->hidden($undefined); } catch (Error $e) { echo $e->getMessage(), "\n"; }
// An undefined receiver: its warning, then the error; no argument runs.
try { $missing->m(eff()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { "text"->m(eff()); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// The same in a function (its variables are not globals), each call site
// run twice.
function local() {
    for ($i = 0; $i < 2; $i++) {
        $o = new A; $v = "v";
        echo $o->m($v, $o = new B), " v=$v ", get_class($o), "\n";
        $o = new A; $v = "v";
        echo $o->m($v, swap($o)), " v=$v ", get_class($o), "\n";
        $o = new B; $v = "v";
        echo $o->m($v, $o = new A), " v=$v ", get_class($o), "\n";
        $n = null; $k = 0;
        try { $n->m($k, $k = 5); } catch (Error $e) { echo $e->getMessage(), " k=$k\n"; }
        $t = new T;
        try { $t->nope(eff()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        try { $t->hidden(eff()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        try { $n->m($undefined); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        try { $t->nope($undefined); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        try { $missing->m(eff()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    }
}
local();

// The receiver is an expression's result: found once, before the arguments.
function make() { echo "make "; return new A; }
echo make()->m(eff(), eff()), "\n";
// A property's and an element's object, replaced by an argument.
class Holder { public $p; public $list = []; }
$h = new Holder; $h->p = new A; $v = "v";
echo $h->p->m($v, $h->p = new B), " v=$v ", get_class($h->p), "\n";
$h->list[0] = new A; $v = "v";
echo $h->list[0]->m($v, $h->list[0] = new B), " v=$v ", get_class($h->list[0]), "\n";
// Calls inside the arguments of a call: each has its own receiver.
$o = new A; $p = new B; $v = "v"; $w = "w";
echo $o->m($v, $p->m($w, $o = new B)), " v=$v w=$w\n";
