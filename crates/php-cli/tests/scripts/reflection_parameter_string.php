<?php
namespace N;
const C = 3;
class K { const D = 'd'; }
function f(int $a, ?string $b = null, $c = 'x\ny', array $d = [], array $e = [1, 2], array $g = ['k' => true, 5 => 1.5], &$h = C, int|bool $i = K::D, ...$rest) {}
foreach ((new \ReflectionFunction('N\f'))->getParameters() as $p) {
    echo $p, "\n";
}
function g(?K $x = null, $y = \PHP_INT_MAX) {}
foreach ((new \ReflectionFunction('N\g'))->getParameters() as $p) {
    echo (string) $p, "\n";
}
echo new \ReflectionParameter('str_pad', 2), "\n";
echo new \ReflectionParameter('array_map', 0), "\n";
var_dump(php_ini_loaded_file(), php_ini_scanned_files());
class M { const Q = 1; function m($a = self::Q, $b = K::D) {} }
foreach ((new \ReflectionMethod(M::class, 'm'))->getParameters() as $p) {
    echo $p, "\n";
}
