<?php
interface I {}
class A implements I { function __toString(): string { return "a"; } }
class B extends A {}
trait T { function __toString(): string { return "t"; } }
class C { use T; }
interface J { function __toString(): string; }
class D {}
var_dump(new A instanceof Stringable, new B instanceof Stringable, new C instanceof Stringable, new D instanceof Stringable);
var_dump(class_implements('A'), class_implements('C'), is_subclass_of('J', 'Stringable'));
function f(string|Stringable $s) { return get_debug_type($s); }
var_dump(f(new C));
var_dump((new ReflectionClass('A'))->getInterfaceNames());
