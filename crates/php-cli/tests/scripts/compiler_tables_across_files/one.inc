<?php
function one($in) {
    $a = 'alpha';
    $b = 'beta';
    $c = 'gamma';
    seen("one: $a $b $c $in");
    $o = Alpha::make();
    return $o->name() . ',' . strtoupper($a) . ',' . str_repeat($b, 2) . ',' . 12 . ',' . 1.5;
}
echo 'file one: ', 'alpha', '|', 'beta', '|', 7, "\n";
