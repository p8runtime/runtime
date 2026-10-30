<?php
// global $$name and $$name = &$v: a variable named at run time bound as a
// reference (MediaWiki's TestSetup imports every global this way).
$wgA = 1; $wgB = [1]; $other = 'o';
function imp() {
    foreach (['wgA', 'wgB', 'wgNew'] as $key) { global $$key; }
    $wgA++; $wgB[] = 2; $wgNew = 'made';
    var_dump(isset($other));
}
imp();
var_dump($wgA, $wgB, $wgNew);
class S { static function load() { foreach ($GLOBALS as $k => $_) { global $$k; } return $wgA + 10; } }
var_dump(S::load());
$y = 1; $n = 'x'; $$n = &$y; $y = 2; var_dump($x);
function loc() { $v = 5; $name = 'alias'; $$name = &$v; $v = 6; return $alias; }
var_dump(loc());
function dyn() { $name = 'zz' . 'top'; $$name = &$GLOBALS['wgA']; $zztop = 100; }
dyn(); var_dump($wgA);
$k = 'wgA'; global $$k; var_dump($wgA);
