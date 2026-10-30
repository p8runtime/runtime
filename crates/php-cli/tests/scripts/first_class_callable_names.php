<?php
// First-class callables resolve names as calls do: use imports, use function,
// and in a namespace ns before the global f.
namespace A\B { class C { static function s($x) { return "s:$x"; } function m($x) { return "m:$x"; } } function f($x) { return "f:$x"; } }
namespace Main {
use A\B\C;
use A\B as AB;
use function A\B\f;
$s = C::s(...); var_dump($s(1));
$s2 = AB\C::s(...); var_dump($s2(2));
$f = f(...); var_dump($f(3));
$m = (new C)->m(...); var_dump($m(4));
$st = strlen(...); var_dump($st('abcd'));
}
