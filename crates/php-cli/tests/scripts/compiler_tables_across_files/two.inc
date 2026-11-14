<?php
function two($in) {
    $c = 'beta';
    $z = 'zeta';
    $b = 'alpha';
    $a = 'gamma';
    seen("two: $a $b $c $z $in");
    $o = Beta::make();
    return str_repeat($c, 2) . ',' . strtoupper($z) . ',' . $o->name() . ',' . 1.5 . ',' . 12 . ',' . true;
}
echo 'file two: ', 'gamma', '|', 7, '|', 'alpha', '|', 'beta', "\n";
