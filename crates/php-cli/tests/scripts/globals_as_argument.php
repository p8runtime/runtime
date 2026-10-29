<?php
// $GLOBALS as a whole passed to a callee the compiler cannot see (a
// method, a callable): its value, not an undefined variable "GLOBALS".
$link = 1;
class A { static function has($k, $a) { return array_key_exists($k, $a); } function ref(&$a) { $a['new'] = 1; } }
class T extends A { function m() {
  var_dump($this->has('link', $GLOBALS));
  var_dump(static::has('link', $GLOBALS));
  $f = 'array_key_exists'; var_dump($f('link', $GLOBALS));
} }
(new T)->m();
function spread(...$a) { return array_key_exists('link', $a[0]); }
var_dump(spread($GLOBALS), call_user_func('array_key_exists', 'link', $GLOBALS), (fn($g) => isset($g['link']))($GLOBALS));
$cb = [new T, 'has']; var_dump($cb('link', $GLOBALS));
