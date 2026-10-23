<?php
// Every comparison as a branch condition (if, ternary, loops), on every
// kind of operand: each compiles to one compare-and-branch instruction,
// with > and >= as swapped < and <=, as PHP compiles them.
$values = [0, 1, -1, 1.5, NAN, INF, "1", "1.0", "abc", "", null, true, false, "10", "9"];
$out = '';
foreach ($values as $a) {
    foreach ($values as $b) {
        $out .= ($a < $b ? 'l' : '-') . ($a <= $b ? 'L' : '-') . ($a > $b ? 'g' : '-') . ($a >= $b ? 'G' : '-');
        $out .= ($a == $b ? 'e' : '-') . ($a != $b ? 'n' : '-') . ($a === $b ? 'i' : '-') . ($a !== $b ? 'N' : '-');
        if ($a < $b) { $out .= '1'; } else { $out .= '0'; }
        if ($a > $b) { $out .= '1'; } else { $out .= '0'; }
        if ($a !== $b) { $out .= '1'; } else { $out .= '0'; }
        $out .= ' ';
    }
    $out .= "\n";
}
echo $out;
// Loops: counting up and down, float steps, and a NaN bound (never true).
for ($i = 0; $i <= 3; $i++) echo $i;
for ($i = 3; $i > 0; $i--) echo $i;
for ($f = 0.0; $f < 1.0; $f += 0.25) echo $f, ' ';
for ($i = 0; $i < NAN; $i++) echo 'never';
$n = 0; while ($n != 4) { $n++; } echo $n, "\n";
$s = 'a'; while ($s !== 'e') { $s++; } echo $s, "\n";
$i = PHP_INT_MAX - 1; $i++; var_dump($i); $i++; var_dump($i);
$j = PHP_INT_MIN + 1; $j--; var_dump($j); $j--; var_dump($j);
// Undefined operands warn in PHP's order: > reads its right operand first.
if ($undefined_left > $undefined_right) echo "no\n";
if ($undefined_a < $undefined_b) echo "no\n";
echo "done\n";
