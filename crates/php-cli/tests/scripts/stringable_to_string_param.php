<?php
class A { function __toString(): string { return "a!"; } }
class B {}
class C { function set(string $x, string|A $y, int|string $z = 0, ?string $n = null) { var_dump($x, $y, $z, $n); } }
function f(string $s) { var_dump($s); }
function g(string ...$s) { var_dump($s); }
f(new A);
(new C)->set(new A, new A, new A, new A);
g(new A, 'x');
try { f(new B); } catch (TypeError $e) { echo explode(', called', $e->getMessage())[0], "\n"; }
function h(int $i) {}
try { h(new A); } catch (TypeError $e) { echo explode(', called', $e->getMessage())[0], "\n"; }
