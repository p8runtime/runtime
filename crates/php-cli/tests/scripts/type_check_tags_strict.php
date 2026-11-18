<?php
declare(strict_types=1);
// type_check_tags.php's parameter and return types under strict_types:
// each value its type takes as it is passes, an int widens to float, and
// every other value is a TypeError (no conversion), again and again.
interface Shape {}
class Base implements Shape {}
class Sub extends Base {}
class Other {}

function p_int(int $x) { return $x; }
function p_float(float $x) { return $x; }
function p_string(string $x) { return $x; }
function p_bool(bool $x) { return $x; }
function p_array(array $x) { return $x; }
function p_nint(?int $x) { return $x; }
function p_union(int|string $x) { return $x; }
function p_fs(float|string $x) { return $x; }
function p_mixed(mixed $x) { return $x; }
function p_iter(iterable $x) { return $x; }
function p_call(callable $x) { return $x; }
function p_object(object $x) { return $x; }
function p_true(true $x) { return $x; }
function p_false(false $x) { return $x; }
function p_base(Base $x) { return $x; }
function p_shape(?Shape $x) { return $x; }
function p_scalar_or_obj(int|Base|null $x) { return $x; }

function r_int($x): int { return $x; }
function r_float($x): float { return $x; }
function r_string($x): string { return $x; }
function r_bool($x): bool { return $x; }
function r_nint($x): ?int { return $x; }
function r_union($x): int|string { return $x; }
function r_mixed($x): mixed { return $x; }
function r_base($x): Base { return $x; }
function r_shape($x): ?Shape { return $x; }

function show($v) {
    if (is_object($v)) return get_class($v);
    if (is_array($v)) return 'array(' . count($v) . ')';
    return var_export($v, true);
}

$heap = str_repeat("4", 2);
$values = [
    'null' => null, 'true' => true, 'false' => false, 'int' => 5, 'float' => 1.5, 'numeric' => "7",
    'heap numeric' => $heap, 'word' => "abc", 'array' => [1, 2], 'base' => new Base, 'sub' => new Sub,
    'other' => new Other, 'closure' => fn() => 1, 'callable name' => 'strlen',
];
$funcs = ['p_int', 'p_float', 'p_string', 'p_bool', 'p_array', 'p_nint', 'p_union', 'p_fs', 'p_mixed', 'p_iter', 'p_call',
    'p_object', 'p_true', 'p_false', 'p_base', 'p_shape', 'p_scalar_or_obj',
    'r_int', 'r_float', 'r_string', 'r_bool', 'r_nint', 'r_union', 'r_mixed', 'r_base', 'r_shape'];
for ($round = 0; $round < 2; $round++) {
    echo "== round $round\n";
    foreach ($funcs as $f) {
        $line = [];
        foreach ($values as $name => $v) {
            try {
                $line[] = "$name=" . show($f($v));
            } catch (TypeError $e) {
                $line[] = "$name: TypeError";
                if ($round == 0 && ($name == 'numeric' || $name == 'int')) echo "  ", $e->getMessage(), "\n";
            }
        }
        echo $f, ": ", implode(", ", $line), "\n";
    }
}

function some(int $a, $b, string $c = "c", ?Base $d = null) { return show($a) . show($b) . show($c) . show($d); }
function by_ref(float &$x) { return show($x); }
function ints(int ...$xs) { return implode(",", $xs); }
for ($i = 0; $i < 3; $i++) {
    $n = 3;
    echo some(1, "b"), " ", some(2, 2, "x", new Sub), " ", by_ref($n), show($n), " ", ints(1, 2, 3), "\n";
    try { some("1", 1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
    try { ints(1, 2.0); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}
