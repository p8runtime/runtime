<?php
declare(strict_types=1);
// property_write_tags.php's typed writes under strict_types: what the type
// takes as it is, int widened to float, and the TypeError for every other
// kind (no conversion), each site run more than once so its cache entry
// answers; `$o->p = v` and `$this->p = v`.
interface Shape {}
class Base implements Shape {}
class Sub extends Base {}
class Other {}
class Str { public function __toString(): string { return "str"; } }

class T {
    public int $p_int = 0;
    public float $p_float = 0.0;
    public string $p_string = '';
    public bool $p_bool = false;
    public array $p_array = [];
    public ?int $p_nint = null;
    public int|string $p_union = 0;
    public float|string $p_fs = 0.0;
    public mixed $p_mixed = null;
    public ?Base $p_base = null;
    public Shape $p_shape;
    public int|float $p_num = 0;
    public readonly float $p_ro;
    public function setInt($v) { $this->p_int = $v; return $this->p_int; }
    public function setFloat($v) { $this->p_float = $v; return $this->p_float; }
    public function setRo($v) { $this->p_ro = $v; return $this->p_ro; }
}

function show(mixed $v): string {
    return get_debug_type($v) . (is_scalar($v) || $v === null ? ' ' . var_export($v, true) : '');
}

function w_int($o, $v) { $o->p_int = $v; return $o->p_int; }
function w_float($o, $v) { $o->p_float = $v; return $o->p_float; }
function w_string($o, $v) { $o->p_string = $v; return $o->p_string; }
function w_bool($o, $v) { $o->p_bool = $v; return $o->p_bool; }
function w_array($o, $v) { $o->p_array = $v; return $o->p_array; }
function w_nint($o, $v) { $o->p_nint = $v; return $o->p_nint; }
function w_union($o, $v) { $o->p_union = $v; return $o->p_union; }
function w_fs($o, $v) { $o->p_fs = $v; return $o->p_fs; }
function w_mixed($o, $v) { $o->p_mixed = $v; return $o->p_mixed; }
function w_base($o, $v) { $o->p_base = $v; return $o->p_base; }
function w_shape($o, $v) { $o->p_shape = $v; return $o->p_shape; }
function w_num($o, $v) { $o->p_num = $v; return $o->p_num; }
function w_this_int($o, $v) { return $o->setInt($v); }
function w_this_float($o, $v) { return $o->setFloat($v); }
function w_ro($o, $v) { return (new T)->setRo($v); }

$values = [7, 1.5, 2.0, "12", "abc", str_repeat("9", 2), true, false, null, [1], new Base, new Sub, new Other, new Str];
$writers = ['w_int', 'w_float', 'w_string', 'w_bool', 'w_array', 'w_nint', 'w_union', 'w_fs', 'w_mixed', 'w_base',
    'w_shape', 'w_num', 'w_this_int', 'w_this_float', 'w_ro'];

$o = new T;
foreach ($writers as $w) {
    echo "== $w\n";
    foreach ($values as $v) {
        $line = [];
        for ($i = 0; $i < 2; $i++) {
            try {
                $line[] = show($w($o, $v));
            } catch (TypeError $e) {
                $line[] = 'TypeError: ' . $e->getMessage();
            }
        }
        echo show($v), ' -> ', $line[0] === $line[1] ? $line[0] : implode(' | ', $line), "\n";
    }
}
