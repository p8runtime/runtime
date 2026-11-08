<?php
// A frame slot keeps the class handles of the last method frame that used
// it (the next call of the same method at that depth reuses them without a
// reference count). What runs next at the same depth must not see them: a
// function or an unbound closure called right after a method has no scope,
// no called class and no $this, whatever the slot kept.
class A {
    private function secret() { return 'A::secret'; }
    public function viaMethod() { return $this->secret(); }
    public static function make() { return new static; }
    public function where() { return [static::class, self::class, get_class($this)]; }
}
class B extends A {
    private function secret() { return 'B::secret'; }
}
function via_function($o) { return $o->secret(); }
function where_am_i() {
    $out = [];
    foreach (['get_called_class', 'get_class'] as $f) {
        try {
            $out[$f] = $f();
        } catch (Error $e) {
            $out[$f] = $e->getMessage();
        }
    }
    return $out;
}

$a = new A;
$b = new B;
// Depth 1: a method frame of scope A, then a function frame.
echo $a->viaMethod(), "\n";
try {
    echo via_function($a), "\n";
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
// Depth 1 again, alternating classes on the same call site.
foreach ([$a, $b, $a, $b] as $o) {
    echo $o->viaMethod(), "\n";
}
foreach ([$b, $a] as $o) {
    try {
        echo via_function($o), "\n";
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
// static:: and self:: at the same depth as the previous method frame.
var_dump($a->where(), $b->where());
var_dump(get_class(B::make()), get_class(A::make()));
echo $b->where()[0], "\n";
var_dump(where_am_i());
// A closure made at global scope, called where a method frame just was:
// the trace shows no class and no object.
$peek = function () { return debug_backtrace()[0]; };
$a->viaMethod();
$entry = $peek();
var_dump(isset($entry['class']), isset($entry['object']), isset($entry['type']));
// Bound the other way: a closure bound to an object, called where a
// function frame just was.
$bound = Closure::bind(function () { return [self::class, get_class($this), $this->secret()]; }, $b, B::class);
via_function_none();
function via_function_none() { return 1; }
var_dump($bound());
// A generator's frame keeps its classes across resumes, at any depth.
class G {
    private $n = 0;
    public function gen() { while ($this->n < 3) { yield static::class . ':' . $this->n++; } }
}
$g = (new G)->gen();
foreach ($g as $v) {
    $a->viaMethod();
    echo $v, "\n";
}
// A method's $this is released when it returns (its destructor runs as
// in PHP: right after the return, before the caller goes on).
class D {
    public function __destruct() { echo "D gone\n"; }
    public function hi() { return 'hi'; }
}
echo (new D)->hi(), " after\n";
// func_get_args() past the parameters, returned from the same slot.
function extra($x) { return array_sum(func_get_args()); }
echo extra(1, 2, 3), ' ', extra(4), ' ', extra(5, 6), "\n";
