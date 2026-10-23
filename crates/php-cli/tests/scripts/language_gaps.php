<?php
$a = ['x' => 0, 'z' => ''];
var_dump(empty($a['x']), empty($a['y']), empty($a), empty($undef), empty($a['z']), empty("0"), empty([1]));
$o = new stdClass; $o->p = 5;
var_dump(empty($o->p), empty($o->q));
$c = null; $c ??= 'dflt'; echo $c, "\n";
$c ??= 'other'; echo $c, "\n";
$arr = []; $arr['k'] ??= 'v'; $arr['k'] ??= 'w'; print_r($arr);
$o->n ??= 3; echo $o->n, "\n";
$n = null;
var_dump($n?->prop, $n?->method(print("not evaluated")));
echo $o?->p, "\n";
$counts = [];
foreach (['a', 'b', 'a'] as $w) { $counts[$w] ??= 0; $counts[$w]++; }
print_r($counts);
$m = [1, 2]; $m[0]++; ++$m[1]; $m[1]--; $x = $m[0]++; echo "$x {$m[0]} {$m[1]}\n";
$deep = []; $deep['a']['b'] = 1; $deep['a']['b'] += 1; $deep['a']['b']++; echo $deep['a']['b'], "\n";
$miss = []; $miss['nope']++; var_dump($miss);
function sum(...$nums) { return array_sum($nums); }
$args = [1, 2, 3];
echo sum(...$args), " ", sum(0, ...$args, ...[10]), "\n";
echo max(...[4, 9, 2]), "\n";
$merged = [...[1, 2], ...['a' => 'A'], 3, ...[4]]; print_r($merged);
function gen() { yield 'k' => 1; yield 2; } print_r([...gen()]);
class S { static function m(...$a) { return implode("-", $a); } function i($a, $b) { return $a . $b; } }
echo S::m(...['x', 'y']), " ", (new S)->i(...['p', 'q']), "\n";
$r = eval('return 1 + 2;'); echo $r, "\n";
eval('$fromEval = "set";'); echo $fromEval, "\n";
var_dump(eval('echo "side\n";'));
eval('function evalFn() { return "ef"; }'); echo evalFn(), "\n";
try { eval('this is not php'); } catch (ParseError $e) { echo get_class($e), "\n"; }
