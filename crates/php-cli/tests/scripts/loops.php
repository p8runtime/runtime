<?php
// Loop steps and tests: ++/-- followed by <, <=, >, >= on the same variable.
for ($i = 0; $i < 5; $i++) { echo $i; }
echo "\n";
for ($i = 0; $i <= 5; ++$i) { echo $i; }
echo "\n";
for ($i = 5; $i > 0; $i--) { echo $i; }
echo "\n";
for ($i = 5; $i >= 0; --$i) { echo $i; }
echo "\n";
for ($i = 0; $i < 2.5; $i++) { echo $i; }
echo "\n";
for ($i = 0; $i < "3"; $i++) { echo $i; }
echo "\n";
for ($i = 0; $i < null; $i++) { echo "never"; }
for ($i = 0.5; $i < 3; $i++) { echo $i, " "; }
echo "\n";
for ($i = "a"; $i < "e"; $i++) { echo $i; }
echo "\n";
for ($i = 0; $i < 10; $i++) { if ($i == 2) { $i = 7; } echo $i; }
echo "\n";
$n = 3;
for ($i = 0; $i < $n; $i++) { if ($i == 1) { $n = 5; } echo $i; }
echo "\n";
$i = PHP_INT_MAX - 2;
for (; $i < PHP_INT_MAX; $i++) { echo $i, "\n"; }
var_dump($i);
$i++;
var_dump($i);
$j = PHP_INT_MIN + 1;
for (; $j > PHP_INT_MIN; $j--) { echo $j, "\n"; }
$j--;
var_dump($j);
$k = 0;
while ($k < 3) { echo $k; $k++; }
echo "\n";
for ($a = 0; $a < 3; $a++) { for ($b = 3; $b > $a; $b--) { echo "$a$b "; } }
echo "\n";
for ($i = 0; $i < 3; $i++) { echo $i; if ($i == 1) { continue; } }
echo "\n";
for ($i = 0; $i < 100; $i++) { if ($i == 4) { break; } }
var_dump($i);
