<?php
$o = new stdClass;
$r = WeakReference::create($o);
var_dump($r === WeakReference::create($o), $r->get() === $o);
unset($o);
var_dump($r->get());
try { new WeakReference; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { clone $r; } catch (Error $e) { echo $e->getMessage(), "\n"; }

$m = new WeakMap;
$a = new stdClass; $b = new ArrayObject;
$m[$a] = 'A'; $m[$b] = [1, 2];
var_dump(count($m), $m[$a], isset($m[$b]), isset($m[new stdClass]));
foreach ($m as $k => $v) echo get_class($k), " => ", json_encode($v), "\n";
var_dump($m);
$c = clone $m;
unset($a);
var_dump(count($m), count($c));
unset($m[$b]);
var_dump(count($m), count($c));
try { $m[new stdClass]; } catch (Error $e) { echo str_starts_with($e->getMessage(), 'Object stdClass#') && str_ends_with($e->getMessage(), ' not contained in WeakMap') ? "ok\n" : $e->getMessage(); }
try { $m['x'] = 1; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $m[] = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
