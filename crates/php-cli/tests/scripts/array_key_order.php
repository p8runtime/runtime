<?php
// An element's key is evaluated before its value (#461): in an array
// literal (plain and by-reference elements) and in `yield k => v`; a
// destructuring's keys come before their targets. A variable key is read
// when the element is added, after its value.
function k($n) { echo "k$n "; return $n; }
function v($n) { echo "v$n "; return $n; }

echo "plain: "; $a = [k(1) => v(1), k(2) => v(2)]; echo json_encode($a), "\n";

$x = 5; $y = 6;
echo "ref: "; $b = [k(1) => &$x, k(2) => &$y]; $b[1] = 50; echo $x, ' ', json_encode($b), "\n";

$arr = [5 => [7 => 'deep']];
echo "refdim: "; $c = [k(1) => &$arr[v(5)][v(7)]]; $c[1] = 'set'; echo json_encode($c), ' ', $arr[5][7], "\n";

echo "list: "; [k(1) => $p, k(2) => $q] = [1 => 'one', 2 => 'two']; echo $p, ' ', $q, "\n";

$src = [1 => 'one', 2 => 'two'];
$t = [];
echo "listdim: "; [k(1) => $t[v(10)], k(2) => $t[v(20)]] = $src; echo json_encode($t), "\n";

function g() { yield k(1) => v(1); yield k(2) => v(2); }
echo "yield:"; foreach (g() as $kk => $vv) { echo " [$kk=>$vv]"; } echo "\n";

$n = 1;
$d = [$n => ++$n, $n => $n * 10];
echo "variable key: ", json_encode($d), "\n";
