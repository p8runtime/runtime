<?php
$arr = [1]; $ref =& $arr[0]; unset($ref); var_dump($arr[0] + ($arr[0] = 2));
$b = ['x' => ['y' => 1]]; $r =& $b['x']['y']; var_dump($b['x']['y'] + ($r = 5), $b['x']['y']);
$c = [1, 2, 3]; foreach ($c as &$v) {} var_dump($c[2] + ($v = 10), json_encode($c));
$d = [0]; $rr = &$d[0]; $copy = $d[0]; $rr = 9; var_dump($copy, $d[0]);
error_reporting(E_ALL & ~E_WARNING); var_dump($d[5], $d["k"]);
