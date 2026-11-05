<?php
// array_first() / array_last() (PHP 8.5): the first or last value,
// dereferenced, or null for an empty array.
var_dump(array_first([]), array_last([]));
var_dump(array_first([1, 2, 3]), array_last([1, 2, 3]));
var_dump(array_first(['b' => 'x', 'a' => 'y']), array_last(['b' => 'x', 'a' => 'y']));
$holes = [5 => 'five', 9 => 'nine', 2 => 'two'];
unset($holes[5]);
var_dump(array_first($holes), array_last($holes));
var_dump(array_first([null, 1]), array_last([1, null]));

// A reference element comes back as its value, not as a reference.
$v = 1;
$a = [&$v, 2];
$first = array_first($a);
$first = 99;
var_dump($v, $a[0]);
$b = [1, &$v];
$last = array_last($b);
$last = 7;
var_dump($v);

// The array argument is unchanged; objects come back as the same instance.
$o = new stdClass;
$c = [$o, [1, 2]];
var_dump(array_first($c) === $o, array_last($c));

// Arguments: exactly one array.
foreach ([fn() => array_first('x'), fn() => array_last(null), fn() => array_first(), fn() => array_last([], [])] as $f) {
    try {
        $f();
    } catch (TypeError $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}

$r = new ReflectionFunction('array_first');
echo $r->getNumberOfParameters(), ' ', $r->getParameters()[0]->getType(), ' ', $r->getReturnType(), "\n";
var_dump(function_exists('array_last'));
