<?php
trait T {
    private const F = ['a'];
    protected const P = 2;
    function a() { return self::F; }
    function b() { return array_filter(['x', 'a'], fn ($v) => in_array($v, [...self::F], true)); }
    static function c() { return static::P; }
}
class S { use T; function own() { return self::F; } }
class Sub extends S { function p() { return self::P; } }
$s = new S;
var_dump($s->a(), $s->b(), S::c(), $s->own(), (new Sub)->p());
var_dump((new ReflectionClassConstant('S', 'F'))->getDeclaringClass()->getName());
try { var_dump(S::F); } catch (Error $e) { echo $e->getMessage(), "\n"; }
