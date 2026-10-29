<?php
// Protected access is checked against the top class declaring the method
// (zend_get_function_root_class): two subclasses of the class that first
// declared it may call each other's override (WordPress's REST
// autosave and revision controllers do).
class Base { protected function m() { return 'Base::m'; } protected static function s() { return 'Base::s'; } protected $p = 'p'; }
class A extends Base { protected function m() { return 'A::m'; } protected static function s() { return 'A::s'; } }
class B extends Base { function call(A $a) { return [$a->m(), A::s()]; } }
var_dump((new B)->call(new A));
class Other { function call(A $a) { try { $a->m(); } catch (Error $e) { echo $e->getMessage(), "\n"; } } }
(new Other)->call(new A);
class C2 extends A { protected function only() { return 'only'; } }
class D2 extends A { function call(C2 $c) { try { return $c->only(); } catch (Error $e) { return $e->getMessage(); } } }
var_dump((new D2)->call(new C2));
