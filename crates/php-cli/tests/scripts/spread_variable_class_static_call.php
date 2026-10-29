<?php
// $class::m(...$args): the class the variable holds (WooCommerce's
// LegacyProxy calls $class_name::instance(...$args) so).
namespace Shop;
class Store { static function instance(...$a) { return static::class . '(' . implode(',', $a) . ')'; } static function &ref(&$x) { $x++; return $x; } }
class Sub extends Store {}
$class_name = 'Shop\Sub';
$args = [1, 2];
var_dump($class_name::instance(...$args), $class_name::instance(0, ...$args));
$o = new Sub;
var_dump($o::instance(...[3]));
$n = 5;
var_dump(Store::ref(...[&$n]), $n);
