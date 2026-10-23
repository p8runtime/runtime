<?php
$arr = [];
$arr[][] = 2;
$aa[][][0] = 2;
$m = ['k' => []];
$m['k'][]['x'][] = 'v';
$m['k'][][1] = 1;
var_dump($arr, $aa, $m);
$c = [];
var_dump($c[] += 5, $c[] .= 0, $c[] .= "ok", $c);
$t = [];
$t[] &= 3; $t[] -= 2;
var_dump($t);
class P { public $p = []; static $s = []; }
$o = new P;
$o->p[][] = 'prop';
P::$s[]['a'] = 'static';
P::$s[] .= 'x';
var_dump($o, P::$s);
$y = [];
var_dump($y = $y[] = 4);
