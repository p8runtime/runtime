<?php
class P { const PC = 'p'; }
class C extends P {
    const X = 'x'; private const PRIV = 'priv';
    static function go() { return [constant('self::X'), constant('static::X'), constant('parent::PC'), constant('self::PRIV'), constant('SELF::X')]; }
}
class D extends C { const X = 'dx'; }
var_dump(C::go(), D::go());
class Q { static function f() { return constant('parent::X'); } }
try { Q::f(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { constant('self::X'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { constant('static::X'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(defined('static::X'));
