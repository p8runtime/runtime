<?php
// ReflectionProperty::setValue / setRawValue store into a typed property as
// an assignment does, in coercive mode whatever this file's strict_types:
// the value converted, or a TypeError and the property left as it was.
declare(strict_types=1);

class IW {
    public int $p = 0;
    public ?float $f = null;
    public string $s = '';
    public int|string $u = 0;
    public bool $b = false;
    private int $priv = 1;
    public readonly int $ro;
    public $plain;
    public ?IW $w = null;
}
class Child extends IW {}

function set(string $class, string $name, object $o, mixed $v, bool $raw = false): void {
    $rp = new ReflectionProperty($class, $name);
    try {
        $raw ? $rp->setRawValue($o, $v) : $rp->setValue($o, $v);
        var_dump($rp->getValue($o));
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
        echo "  kept: ", var_export($rp->getValue($o), true), "\n";
    }
}

$o = new IW;
foreach ([5.0, 5.5, '7', '7.0', ' 8', '8x', 'x', true, null, [], 1e30, NAN, new stdClass, -0.0] as $v) {
    set(IW::class, 'p', $o, $v);
}
foreach ([3, '2.5', 'x', true, null] as $v) {
    set(IW::class, 'f', $o, $v);
}
foreach ([3, 2.5, true, null, new stdClass] as $v) {
    set(IW::class, 's', $o, $v);
}
foreach ([2.0, 2.5, '3', true, 'x'] as $v) {
    set(IW::class, 'u', $o, $v);
}
foreach ([1, 0.0, 'a', '', []] as $v) {
    set(IW::class, 'b', $o, $v);
}
set(IW::class, 'priv', $o, '9');
set(IW::class, 'priv', $o, 'z');
set(IW::class, 'ro', $o, '4');
set(IW::class, 'plain', $o, 1.5);
set(IW::class, 'w', $o, new Child);
set(IW::class, 'w', $o, new stdClass);

// Through a reference to the property.
$r = &$o->p;
set(IW::class, 'p', $o, '12');
var_dump($r);
set(IW::class, 'p', $o, 'zz');
var_dump($r);
unset($r);

set(IW::class, 'p', $o, '13', true);
set(IW::class, 'p', $o, 'w', true);

$c = new Child;
set(Child::class, 'p', $c, '21');
set(Child::class, 'p', $c, []);
var_dump($o, $c);
