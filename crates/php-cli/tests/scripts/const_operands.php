<?php
// Literals read in place by the instructions that take a constant operand
// (compare-and-branch, compare, arithmetic and concatenation on either
// side, op=, a dimension's key or value, array literals, assign, return,
// echo, a one-literal builtin call, a loop's literal bound), and the cases
// around them: a literal that a declared return type converts, a literal
// where the instruction reads a register, a frame whose script has no
// variables, eval'd literals.

function cmp($x) {
    $r = '';
    if ($x < 10) $r .= 'a';
    if (10 < $x) $r .= 'b';
    if ($x <= 10) $r .= 'c';
    if (10 <= $x) $r .= 'd';
    if ($x > 10) $r .= 'e';
    if (10 > $x) $r .= 'f';
    if ($x >= 10) $r .= 'g';
    if (10 >= $x) $r .= 'h';
    if ($x == 10) $r .= 'i';
    if ('10' == $x) $r .= 'j';
    if ($x != 10) $r .= 'k';
    if ($x === 10) $r .= 'l';
    if (10 === $x) $r .= 'm';
    if ($x !== 10) $r .= 'n';
    if (null !== $x) $r .= 'o';
    if ($x === null) $r .= 'p';
    if (!($x < 10)) $r .= 'q';
    if (!(10 < $x)) $r .= 'r';
    return $r;
}
foreach ([3, 10, 10.0, '10', 11, null, 'abc', true] as $v) {
    echo var_export($v, true), ': ', cmp($v), ' ', var_export($v < 10, true), var_export(10 < $v, true), var_export($v == 10, true), var_export(10 === $v, true), var_export($v <=> 10, true), var_export(10 <=> $v, true), "\n";
}

function arith($x) {
    $parts = [$x + 1, 1 + $x, $x - 1, 1 - $x, $x * 2, 2 * $x, $x / 2, 2 / $x, $x % 3, 7 % $x, $x ** 2, 2 ** $x, $x . 'z', 'a' . $x, $x | 1, 1 & $x, $x << 1];
    return implode(' ', array_map(fn($v) => var_export($v, true), $parts));
}
echo arith(5), "\n", arith(2.5), "\n", arith('3'), "\n";
try { echo arith('x'), "\n"; } catch (TypeError $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }

$s = 'p'; $s .= 'q'; $s .= 1; $s .= 2.5; $s .= true;
$n = 5; $n += 2; $n -= 1; $n *= 3; $n .= '0'; $n += 1.5; $n /= 2;
echo $s, ' ', $n, "\n";

$a = ['k' => 1, 2 => 'two', 'n' => null, 3.7 => 'f', true => 'b', 'd' => [1, 2]];
$a['k'] = 'one'; $a['z'] = true; $a[5] = 'five'; $a['d'][1] = 'x'; $a[] = 'pushed';
var_dump($a);
echo $a['k'], ' ', $a[2], ' ', $a['d'][0], ' ', var_export(isset($a['n']), true), var_export(isset($a['k']), true), var_export(isset($a['nope']), true), var_export(isset($a['d'][1]), true), var_export(isset($a[2]), true), "\n";
echo $a['missing'] ?? 'default', ' ', $a['k'] ?? 'default', "\n";
echo @$a['warned'], "|\n";
$str = 'hello';
echo $str[0], $str[-1], ' ', var_export(isset($str[1]), true), var_export(isset($str[9]), true), "\n";
$str[0] = 'J'; echo $str, "\n";

// A loop's literal bound, stepping both ways.
$out = '';
for ($i = 0; $i < 5; $i++) $out .= $i;
for ($i = 5; $i > 0; $i--) $out .= $i;
for ($i = 0; $i <= 3; $i++) $out .= $i;
for ($i = 3; $i >= 0; $i--) $out .= $i;
for ($i = 1.5; $i < 3; $i++) $out .= $i;
echo $out, "\n";

// Literals returned: as they are, and converted by the declared type.
function r1() { return 'lit'; }
function r2(): int { return "7"; }
function r3(): float { return 1; }
function r4(): ?string { return null; }
function r5(): bool { return 1; }
function r6(): string { return 2.5; }
function r7(): int { return true; }
$r8 = fn(): int => "42";
$r9 = fn() => 'arrow';
var_dump(r1(), r2(), r3(), r4(), r5(), r6(), r7(), $r8(), $r9());
function r10(): int { return "x"; }
try { r10(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }

// A one-literal builtin call, and a literal the builtin converts or refuses.
echo strtoupper('lit'), ' ', strrev('abc'), ' ', var_export(function_exists('cmp'), true), var_export(defined('PHP_VERSION'), true), ' ', ucfirst(5), ' ', abs(-3), ' ', count([1, 2]), ' ', strlen('four'), "\n";
try { echo chr([]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
echo intdiv(7, 2), ' ', str_repeat('ab', 2), ' ', max(3, 1), "\n";

// Literals where the instruction reads a register: an unset key, a
// variable's name, a static call's arguments, interpolation, a match.
$k = 'dyn';
$b = [$k => 1, 'lit' => 2, "str$k" => 3];
unset($b['lit']);
var_dump($b, array_key_exists('dyn', $b), in_array(3, $b, true));
class K { const C = 'cc'; static $sp = 'static'; public $p = 'prop'; static function sf($x, $y) { return "$x-$y"; } function m() { return $this->p . self::C . static::$sp; } }
echo K::sf('lit', 2), ' ', (new K)->m(), ' ', K::C, ' ', K::$sp, "\n";
echo match (3) { 1, 2 => 'low', 3 => 'three', default => 'other' }, "\n";
$name = 'v'; $$name = 'indirect'; echo $v, "\n";
echo "interp {$a['k']} and $s", "\n";
list('k' => $lk, 2 => $l2) = $a; echo $lk, $l2, "\n";
echo str_contains('haystack', 'st') ? 'yes' : 'no', "\n";
$x = null;
echo $x ?? 'nullish', ' ', $x ?: 'falsy', ' ', (int) '12abc', ' ', (bool) 'x', "\n";

// Eval'd literals (counted strings, not program memory).
echo eval('return "evaluated" . 1;'), ' ', eval('$e = "e"; $e .= "v"; return $e;'), "\n";

// An included file's literals, and a frame of a script with no variables.
file_put_contents(__DIR__ . '/const_operands.inc', '<?php function inc_f() { $t = "t"; $t .= "u"; return $t . 1; } return inc_f() . "!";');
echo include __DIR__ . '/const_operands.inc', "\n";
unlink(__DIR__ . '/const_operands.inc');

// Static initializers and defaults are literals too.
function st($d = 'dflt', $n = 7) { static $c = 10; $c++; return "$d $n $c"; }
echo st(), ' ', st('x'), ' ', st('y', 1), "\n";
class V { public int $i = 1; public ?string $s = null; public array $arr = ['a' => 'b']; public $f = 2.5; }
$o = new V; $o->i = 5; $o->s = 'str'; $o->arr['c'] = 'd';
var_dump($o);

// A literal as a class name: the reference reads a register.
echo ('K')::sf('lit', 'class'), ' ', (new ('K'))->p, ' ', var_export($o instanceof ('V'), true), "\n";
