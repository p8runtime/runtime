<?php
// Sorting as zend_sort does: PHP's comparison of mixed values is not a
// total order, so the order a sort leaves (and the pairs a usort callback
// sees) depend on the algorithm. p8 sorts with zend_sort's steps: PHP's
// results, and never a panic (WordPress's assertSameSets sorts post rows).
$mixed = ['10', '9a', '9', 10, '1e1', 'abc', true, null, 0, '0', '', [1], 'Z', 9.5, '9.5x'];
$a = $mixed; sort($a); var_dump($a);
$a = $mixed; rsort($a); echo json_encode($a), "\n";
$a = $mixed; asort($a); echo json_encode(array_keys($a)), "\n";
$a = ['b' => 1, 'a' => 1, 'c' => 0, 'd' => 1]; asort($a); echo json_encode(array_keys($a)), "\n";
$calls = [];
$u = [5, 3, 8, 1, 9, 2, 7, 4, 6, 0, 11, 15, 13, 12, 14, 10, 19, 17, 18, 16, 20];
usort($u, function ($x, $y) use (&$calls) { $calls[] = "$x-$y"; return $x <=> $y; });
echo implode(',', $u), "\n", count($calls), ' ', implode(' ', array_slice($calls, 0, 30)), "\n";
$bad = range(0, 40);
usort($bad, function ($x, $y) { return (($x * 7 + $y * 3) % 5) - 2; });
echo implode(',', $bad), "\n";
$big = [];
for ($i = 0; $i < 2000; $i++) { $big[] = ($i * 7919) % 2003 . ($i % 3 ? 'x' : ''); }
sort($big); echo md5(json_encode($big)), "\n";
