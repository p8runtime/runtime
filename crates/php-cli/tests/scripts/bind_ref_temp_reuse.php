<?php
// A reference bound from a temporary must not be written through when the
// temporary's register is reused (MediaWiki's PathMatcher).
$o = new stdClass; $o->t = [];
$parts = ['', 'a', 'b'];
$length = 3;
$tree =& $o->t[$length];
for ($i = 1; $i < $length - 1; $i++) {
    if ($parts[$i] == '') { throw new Exception('x'); }
}
foreach ($parts as $part) {
    $key = "=$part";
    if (!isset($tree[$key])) { $tree[$key] = []; }
    $tree =& $tree[$key];
}
var_dump($o->t);
$a = [1, 2]; $b = [3, 4];
$a[0] = &$b[1];
$x = $parts[1] . 'z';
$y = count($parts) - 1;
var_dump($a, $b, $x, $y);
$o->p = &$b[0];
$z = $parts[2] . 'q';
var_dump($b, $z);
