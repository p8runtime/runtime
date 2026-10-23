<?php
class P { public $p1 = 1; protected $p2 = [1, 2]; private $p3 = 'x'; public static $ps = 'static'; private static $pps = 2; }
class C extends P {
    public $c1 = null; public int $typed; public ?int $typedDefault = 5; private $p3 = 'own';
    const K = 3; public $fromConst = self::K * 2; public static $cs = 'cs';
    static function inside() { return get_class_vars('C'); }
}
var_dump(get_class_vars('C'));
var_dump(C::inside());
var_dump(get_class_vars('P'));
var_dump(get_class_vars('Nope'));
C::$cs = 'changed';
var_dump(get_class_vars('C')['cs']);
