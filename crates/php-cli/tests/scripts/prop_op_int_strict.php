<?php
declare(strict_types=1);
// prop_op_int.php's typed cases under strict_types: the result of
// `$o->p += y` is assigned to the property as this file's mode says (a float
// is never an int; an int is a float).
class T { public int $p = 0; }
class N { public ?int $p = 0; }
class IntStr { public int|string $p = 0; }
class IntFloat { public int|float $p = 0; }
class F { public float $p = 0.0; }
class FD { public float $p = 1; }

function add($o, $y) { return $o->p += $y; }
function sub($o, $y) { return $o->p -= $y; }
function mul($o, $y) { return $o->p *= $y; }
function show($what, $f) {
    try {
        $r = $f();
        echo $what, ': ';
        var_dump($r);
    } catch (Throwable $e) {
        echo $what, ': ', get_class($e), ': ', $e->getMessage(), "\n";
    }
}

for ($i = 0; $i < 3; $i++) {
    foreach (['T', 'N', 'IntStr', 'IntFloat', 'F', 'FD'] as $c) {
        echo "$c\n";
        foreach ([
            '5 + 3' => [5, 'add', 3],
            '5 - 8' => [5, 'sub', 8],
            '5 * -3' => [5, 'mul', -3],
            '5 + 1.5' => [5, 'add', 1.5],
            '5 + 2.0' => [5, 'add', 2.0],
            '5 + "5"' => [5, 'add', '5'],
            '5 * "1.5"' => [5, 'mul', '1.5'],
            '5 + true' => [5, 'add', true],
            '5 - null' => [5, 'sub', null],
            'max + 1' => [PHP_INT_MAX, 'add', 1],
            'min - 1' => [PHP_INT_MIN, 'sub', 1],
            'max * 2' => [PHP_INT_MAX, 'mul', 2],
            'min * -1' => [PHP_INT_MIN, 'mul', -1],
            'max-1 + 1' => [PHP_INT_MAX - 1, 'add', 1],
        ] as $what => [$start, $f, $y]) {
            $o = new $c;
            $o->p = $start;
            show($what, fn() => $f($o, $y));
            echo '  left: ';
            var_dump($o->p);
        }
    }
}
