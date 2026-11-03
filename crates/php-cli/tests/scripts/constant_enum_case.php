<?php
enum L: int { case Debug = 100; case Info = 200; const X = 5; const Y = self::X * 2; const Z = self::Info; }
var_dump(constant('L::Debug'), constant('L::X'), constant('L::Y'), constant('L::Z'));
var_dump((new ReflectionClassConstant('L', 'Debug'))->getValue());
var_dump((new ReflectionClass('L'))->getConstants());
var_dump((new ReflectionClass('L'))->getConstant('Info'));
class C { const A = 1 << 3; }
var_dump(constant('C::A'));
var_dump(is_int(gc_mem_caches()));
