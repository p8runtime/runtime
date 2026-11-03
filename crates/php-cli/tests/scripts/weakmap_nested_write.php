<?php
$m = new WeakMap; $o = new stdClass;
$m[$o] = [];
$m[$o][] = 1;
$m[$o]['k'] = 2;
var_dump($m[$o]);
$m[$o]['n']['x'] = 3; var_dump($m[$o]['n']);
$m[$o]['k']++; var_dump($m[$o]['k']);
$m[$o]['k'] += 10; var_dump($m[$o]['k']);
unset($m[$o]['n']); var_dump(array_keys($m[$o]));
$m[$o]['obj'] = new ArrayObject([]); $m[$o]['obj']['z'] = 9; var_dump($m[$o]['obj']['z']);
$p = new stdClass;
try { $m[$p][] = 'auto'; } catch (Error $e) { echo get_class($e), ': ', preg_replace('/#\\d+/', '#N', $e->getMessage()), "\n"; }

class A implements ArrayAccess { public $d = ["a" => ["b" => 1]]; function offsetGet($k): mixed { return $this->d[$k]; } function offsetSet($k, $v): void {} function offsetExists($k): bool { return true; } function offsetUnset($k): void {} }
$a = new A; unset($a["a"]["b"]); echo "user done\n";
$ao = new ArrayObject(["a" => ["b" => 1]]); unset($ao["a"]["b"]); var_dump(count($ao["a"]));
