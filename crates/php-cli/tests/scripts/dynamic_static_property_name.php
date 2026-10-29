<?php
// C::${$name} and C::$$name: a static property named at run time, read and in isset (Yoast reads its models table name so).
namespace Yoast\M;
class Model { public static $table = null; }
class Indexable extends Model { public static $table = 'indexable'; public static $other; }
$c = 'Yoast\M\Indexable'; $p = 'table'; $q = 'other';
var_dump(\class_exists($c), \property_exists($c, $p), \property_exists($c, 'nope'));
var_dump(isset($c::${$p}), $c::${$p}, isset($c::${$q}), isset($c::$table), $c::$table);
$name = 'table'; var_dump(Indexable::${$name}, isset(Indexable::${$name}), isset(Indexable::${'tab' . 'le'}));
