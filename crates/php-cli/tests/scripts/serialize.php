<?php
class A { public $a = 1; protected $b = [1.5, true]; private $c = "x"; public ?int $u; }
$o = new A; $arr = [$o, $o, 'k' => null, 0.1, -0.0, 1.0];
echo str_replace("\0", "~", serialize($arr)), "\n";
var_dump(unserialize(serialize($arr)));
var_dump(unserialize('a:1:{i:0;'));
var_dump(unserialize('x'));
class S { public $x = 1; public $y = 2; function __sleep() { return ['y']; } function __wakeup() { echo "wakeup\n"; } }
echo serialize(new S), "\n"; var_dump(unserialize(serialize(new S)));
class M { public $p = 5; function __serialize(): array { return ['pp' => $this->p]; } function __unserialize(array $d): void { $this->p = $d['pp'] * 2; } }
echo serialize(new M), "\n"; var_dump(unserialize(serialize(new M)));
var_dump(unserialize('O:7:"Missing":1:{s:1:"a";i:1;}'));
try { serialize(function() {}); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
enum E { case X; } echo serialize(E::X), "\n"; var_dump(unserialize(serialize(E::X)) === E::X);
var_dump(unserialize('a:1:{i:0;O:8:"stdClass":0:{}}', ['allowed_classes' => false]));
