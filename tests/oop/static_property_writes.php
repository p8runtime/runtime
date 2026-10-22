<?php
class R { public static $d = []; public static $c = ['p' => ['m' => null]];
  static function load($p) {
    if (!isset(self::$d['key'])) { echo "Loading\n"; self::$d['key'] = [1,2]; } else echo "Cached\n";
    $col = &self::$c[$p];
    if (null === $col['m']) { echo "L2\n"; $col['m'] = 5; } else echo "C2\n";
  }
}
R::load('p'); R::load('p'); var_dump(R::$d, R::$c);
R::$d['a']['b'][] = 3; var_dump(R::$d['a']);
$a = ['x' => ['y' => 1]]; $r = &$a['x']['y']; $r = 9; echo $a['x']['y'], "\n";
$o = new stdClass; $o->p = ['q' => 1]; $r2 = &$o->p['q']; $r2 = 7; echo $o->p['q'], "\n";
