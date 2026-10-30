<?php
// Abstract methods from two traits meet without collision; a concrete one from a
// third trait (or the class) wins (ta-tikoma/phpunit-architecture-test, Pest arch).
trait A { abstract public static function check($x): void; public function a() { return 'a'; } }
trait B { abstract public static function check($x): void; public function b() { return 'b'; } }
trait C { public static function check($x): void { echo "check $x\n"; } }
trait All { use A; use B; }
class T { use All; public static function check($x): void { echo "own $x\n"; } }
T::check(1); echo (new T)->a(), (new T)->b(), "\n";
class U { use A, B, C; }
U::check(2);
class V { use C, A; }
V::check(3);
var_dump((new ReflectionMethod('U', 'check'))->isAbstract());
