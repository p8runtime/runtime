<?php
// A trait method reached twice (directly and through another trait) is no collision.
trait Mock { public function anon() { return 'anon from ' . self::class; } }
trait Rest { use Mock; public function rest() { return 'rest'; } }
trait Handler { use Rest; use Mock; }
class T { use Handler; }
$t = new T; echo $t->anon(), ' ', $t->rest(), "\n";
trait A { public function hi() { return 'A'; } }
trait B { public function hi() { return 'B'; } }
class C { use A, B { A::hi insteadof B; } }
echo (new C)->hi(), "\n";
