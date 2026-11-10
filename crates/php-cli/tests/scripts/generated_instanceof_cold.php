<?php
// INSTANCEOF (ops.def) and its cold cases: an undefined variable, a
// reference, values that are not objects, a class not declared yet (never
// autoloaded), an alias, interfaces and parents, `self`/`static`/`parent`
// in a class and `self`/`static` in a trait. No `@`; the expected output
// is PHP 8.6's.

spl_autoload_register(function ($c) { echo "autoload($c)\n"; });

interface I {}
interface J extends I {}
class P implements J {}
class C extends P {}
class Other {}

function test($x) {
    return [$x instanceof C, $x instanceof P, $x instanceof I, $x instanceof J, $x instanceof Other, $x instanceof Missing];
}
echo "-- named\n";
echo json_encode(test(new C)), json_encode(test(new P)), json_encode(test(new Other)), "\n";
// Run twice: the second time through the resolved classes.
echo json_encode(test(new C)), json_encode(test(new P)), "\n";
echo json_encode(test(1)), json_encode(test('C')), json_encode(test(null)), json_encode(test([])), "\n";
// A reference, to an object and to a scalar.
$o = new C;
$r = &$o;
$s = 5;
$rs = &$s;
var_dump($r instanceof C, $rs instanceof C);
// An undefined variable: the warning, then false.
var_dump($undef instanceof C);
// A class declared after the code that names it first ran.
function later($x) { return $x instanceof Late; }
var_dump(later(new C));
class Late {}
var_dump(later(new Late), later(new C));
// An alias.
class_alias('C', 'AliasC');
function alias($x) { return $x instanceof AliasC; }
var_dump(alias(new C), alias(new P), alias(new C));
// Closures (Closure is a class) and a dynamic class name.
$f = fn() => 1;
var_dump($f instanceof Closure, $f instanceof C);
$name = 'P';
var_dump(new C instanceof $name, new Other instanceof $name);

echo "-- self, static, parent\n";
class Base {
    public function check($x) { return [$x instanceof self, $x instanceof static]; }
}
class Child extends Base {
    public function up($x) { return $x instanceof parent; }
}
echo json_encode((new Base)->check(new Child)), json_encode((new Child)->check(new Base)), json_encode((new Child)->check(new Child)), "\n";
var_dump((new Child)->up(new Base), (new Child)->up(new Other));
trait T {
    public function mine($x) { return [$x instanceof self, $x instanceof static]; }
}
class UsesT { use T; }
class UsesT2 extends UsesT {}
echo json_encode((new UsesT)->mine(new UsesT2)), json_encode((new UsesT2)->mine(new UsesT)), json_encode((new UsesT)->mine(1)), "\n";
// `static` in a static method: the called class.
class S1 { public static function is($x) { return $x instanceof static; } }
class S2 extends S1 {}
var_dump(S1::is(new S2), S2::is(new S1), S2::is(new S2));
// `self` and `static` with no class: errors.
try {
    $g = function ($x) { return $x instanceof static; };
    var_dump($g(new C));
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
// A closure bound to a class: its `static`.
$h = Closure::bind(function ($x) { return $x instanceof static; }, null, P::class);
var_dump($h(new C), $h(new Other));
