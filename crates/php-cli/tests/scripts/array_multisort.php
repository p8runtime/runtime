<?php
$data = [3, 1, 2, 1];
$other = ['c', 'a', 'b', 'z'];
var_dump(array_multisort($data, $other), $data, $other);
$a1 = [10, 100, 100, 0];
$a2 = [1, 3, 2, 4];
array_multisort($a1, $a2);
var_dump($a1, $a2);
$ar = [["10", 11, 100, 100, "a"], [1, 2, "2", 3, 1]];
array_multisort($ar[0], SORT_ASC, SORT_STRING, $ar[1], SORT_NUMERIC, SORT_DESC);
var_dump($ar);
$keyed = ['x' => 3, 'y' => 1, 5 => 2, 9 => 0];
array_multisort($keyed, SORT_DESC);
var_dump($keyed);
$n = ['img12', 'img10', 'IMG2', 'img1'];
array_multisort($n, SORT_NATURAL | SORT_FLAG_CASE);
var_dump($n);
$s = ['img12', 'img10', 'img2'];
sort($s, SORT_NATURAL);
var_dump($s);
try { array_multisort($data, SORT_ASC, SORT_DESC); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { array_multisort($data, 99); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { array_multisort($data, "x"); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { $short = [1]; array_multisort($data, $short); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$e = []; var_dump(array_multisort($e));
