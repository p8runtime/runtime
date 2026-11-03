<?php
namespace N;
const LOCAL = 1;
class K { const A = 2; function f($a = self::A, $b = K::A, $c = PHP_EOL, $d = LOCAL, $e = \N\LOCAL, $g = 5, $h = self::A + 1, $i = M_PI, $j = UNDEFINED_X) {} }
foreach ((new \ReflectionMethod(K::class, 'f'))->getParameters() as $p) {
    echo $p->getName(), ' ', var_export($p->isDefaultValueConstant(), true), ' ', $p->isDefaultValueConstant() ? $p->getDefaultValueConstantName() : '-', "\n";
}
echo var_export((new \ReflectionMethod(K::class, 'f'))->getParameters()[0]->getDefaultValue(), true), "\n";
echo var_export((new \ReflectionParameter('str_pad', 'pad_type'))->isDefaultValueConstant(), true), ' ', (new \ReflectionParameter('str_pad', 'pad_type'))->getDefaultValueConstantName(), "\n";
