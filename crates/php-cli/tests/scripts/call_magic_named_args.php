<?php
class Q { function withAttributes($a, $v = null, $asConditions = true) { var_dump(func_get_args()); } }
class Rel { public $q; function __construct() { $this->q = new Q; }
  function __call($m, $args) { var_dump($args); return $this->q->{$m}(...$args); }
  static function __callStatic($m, $args) { var_dump($args); } }
(new Rel)->withAttributes(['a' => 1], asConditions: false);
(new Rel)->withAttributes('v', true, asConditions: false);
Rel::stat(1, x: 2);
