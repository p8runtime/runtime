<?php
// Parameter and return types checked against every kind of value, again
// and again (a check is made once per function, then answered by the
// value's kind): what each type takes as it is, what weak mode converts
// (int to float, numeric strings, scalars to string and bool), and the
// TypeError for the rest; untyped parameters beside typed ones, missing
// arguments with defaults, typed references and variadics, objects of a
// named class (the class remembered, then another one), and the same
// types as return types, a generator's included.
interface Shape {}
class Base implements Shape {}
class Sub extends Base {}
class Other {}
class Str { public function __toString(): string { return "str"; } }

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
function p_null(null $x) { return $x; }
function p_base(Base $x) { return $x; }
function p_shape(?Shape $x) { return $x; }
function p_inter(Base&Shape $x) { return $x; }
function p_bs(Base|string $x) { return $x; }
function p_scalar_or_obj(int|Base|null $x) { return $x; }

function r_int($x): int { return $x; }
function r_float($x): float { return $x; }
function r_string($x): string { return $x; }
function r_bool($x): bool { return $x; }
function r_array($x): array { return $x; }
function r_nint($x): ?int { return $x; }
function r_union($x): int|string { return $x; }
function r_mixed($x): mixed { return $x; }
function r_iter($x): iterable { return $x; }
function r_object($x): object { return $x; }
function r_true($x): true { return $x; }
function r_false($x): false { return $x; }
function r_null($x): null { return $x; }
function r_base($x): Base { return $x; }
function r_shape($x): ?Shape { return $x; }
function r_bs($x): Base|string { return $x; }

function show($v) {
    if (is_object($v)) return get_class($v);
    if (is_array($v)) return 'array(' . count($v) . ')';
    return var_export($v, true);
}

$heap = str_repeat("4", 2);
$values = [
    'null' => null, 'true' => true, 'false' => false, 'int' => 5, 'neg' => -3,
    'float' => 1.5, 'whole float' => 2.0, 'numeric' => "7", 'heap numeric' => $heap,
    'float string' => "2.5", 'word' => "abc", 'empty' => "", 'array' => [1, 2],
    'base' => new Base, 'sub' => new Sub, 'other' => new Other, 'str obj' => new Str,
    'closure' => fn() => 1, 'callable name' => 'strlen',
];
$params = ['p_int', 'p_float', 'p_string', 'p_bool', 'p_array', 'p_nint', 'p_union', 'p_fs', 'p_mixed', 'p_iter', 'p_call',
    'p_object', 'p_true', 'p_false', 'p_null', 'p_base', 'p_shape', 'p_inter', 'p_bs', 'p_scalar_or_obj'];
$returns = ['r_int', 'r_float', 'r_string', 'r_bool', 'r_array', 'r_nint', 'r_union', 'r_mixed', 'r_iter', 'r_object',
    'r_true', 'r_false', 'r_null', 'r_base', 'r_shape', 'r_bs'];
for ($round = 0; $round < 2; $round++) {
    echo "== round $round\n";
    foreach (array_merge($params, $returns) as $f) {
        $line = [];
        foreach ($values as $name => $v) {
            try {
                $line[] = "$name=" . show($f($v));
            } catch (TypeError $e) {
                $line[] = "$name: TypeError";
                if ($round == 0 && ($name == 'word' || $name == 'other' || $name == 'array')) echo "  ", $e->getMessage(), "\n";
            }
        }
        echo $f, ": ", implode(", ", $line), "\n";
    }
}

// Untyped parameters among typed ones; defaults for missing arguments.
function some(int $a, $b, string $c = "c", ?Base $d = null, $e = 5) { return show($a) . show($b) . show($c) . show($d) . show($e); }
for ($i = 0; $i < 3; $i++) {
    echo some(1, [3]), " ", some("2", null, 3), " ", some(3, new Other, "x", new Sub), " ", some(4.0, 1, "y", null, 9), "\n";
    try { some(1); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
    try { some("x", 1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
    try { some(1, 2, 3, new Other); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}

// Typed references: the value behind the reference is checked, and converted in place.
function by_ref(int &$x, ?string &$y) { $x++; return show($x) . show($y); }
for ($i = 0; $i < 3; $i++) {
    $a = "5"; $b = 7; $c = 2;
    echo by_ref($a, $b), " ", show($a), show($b), " ", by_ref($c, $nothing), "\n";
    $d = "nope";
    try { by_ref($d, $b); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}

// Variadics: typed (every element checked) and untyped after a typed parameter.
function ints(string $sep, int ...$xs) { return implode($sep, array_map('show', $xs)); }
function rest(int $first, ...$more) { return show($first) . count($more); }
for ($i = 0; $i < 3; $i++) {
    echo ints(",", 1, "2", 3.0), " ", ints("-"), " ", rest("1", [], new Other, "x"), " ", ints(":", ...['a' => 4, 'b' => 5]), "\n";
    try { ints(",", 1, "two"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
    try { rest([]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}

// Methods: the class remembered per parameter, then another accepted, then one refused.
class Box {
    public function put(Base $b, int $n, Shape ...$more): static { return $this; }
    public static function make(?Base $b, float $f): self { return new static; }
    public function self(self $other): self { return $other; }
}
class BigBox extends Box {}
$box = new Box;
foreach ([new Base, new Base, new Sub, new Base, new Other] as $o) {
    try {
        echo get_class($box->put($o, "3", new Sub, new Base)), " ", get_class(Box::make($o, 2)), " ", get_class(BigBox::make($o, 1)),
            " ", get_class($box->self(new BigBox)), "\n";
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
}
try { $box->self(new Base); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }

// Return types of a generator, void, never-returning paths and static.
function gen(): Generator { yield 1; yield 2; }
function gen_iter(): iterable { yield 3; }
function nothing(): void { return; }
function maybe_int(bool $b): ?int { if ($b) return 1; return null; }
function bad_int(): int { return "z"; }
function no_value(): int { if (false) return 1; }
foreach ([1, 2] as $i) {
    echo implode(",", iterator_to_array(gen())), " ", implode(",", iterator_to_array(gen_iter())), " ", show(nothing()), " ",
        show(maybe_int(true)), show(maybe_int(false)), "\n";
    try { bad_int(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
    try { no_value(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}

// Fractional floats to int: the deprecation, and int|string keeps the conversion.
function to_int(int $x) { return $x; }
function to_is(int|string $x): int|string { return $x; }
foreach ([1, 2] as $i) {
    echo show(to_int(3.0)), " ", show(to_is(1.0)), " ", show(to_is(1.5)), "\n";
    echo show(to_int(2.5)), "\n";
}
