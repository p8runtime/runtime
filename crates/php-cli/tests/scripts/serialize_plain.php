<?php
// serialize() of values with no object in them (written from the argument
// alone), and of ones with an object deep in them (written by the call).
$r = 5;
$plain = [
    PHP_INT_MIN, PHP_INT_MAX, -1, 0, 1234567890,
    -7 => 'neg', PHP_INT_MIN => 'min', '' => '', 'é' => "ü\0x",
    'f' => [0.1, -0.0, 1e100, -1.5e-7, NAN, INF, -INF, 2.0],
    'b' => [true, false, null],
    'ref' => [&$r, $r],
    'long' => str_repeat('ab', 600),
    'nest' => [[[]], [[1 => [2 => []]]]],
    'res' => STDIN,
];
echo str_replace("\0", '~', serialize($plain)), "\n";
foreach ([0, -0, 1.0, '', 'x', true, null, [], STDOUT] as $v) {
    echo serialize($v), "\n";
}
class W { public $a = 1; public $b = 2; function __sleep() { echo "sleep\n"; return ['a', 'missing']; } }
echo serialize([1, 'k' => [true, new W], 'after' => 3]), "\n";
class T { function __serialize(): array { throw new RuntimeException('no'); } }
try { serialize(['x' => 1, [new T]]); } catch (RuntimeException $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { serialize([1, [2, function () {}]]); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
$o = new stdClass;
echo serialize([1, $o, [$o]]), "\n";
