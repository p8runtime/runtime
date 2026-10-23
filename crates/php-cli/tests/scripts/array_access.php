<?php
class A implements ArrayAccess, Countable {
    public $d = [];
    function offsetExists($k): bool { echo "exists ", var_export($k, true), "\n"; return isset($this->d[$k]); }
    function offsetGet($k): mixed { echo "get ", var_export($k, true), "\n"; return $this->d[$k] ?? null; }
    function offsetSet($k, $v): void { echo "set ", var_export($k, true), "\n"; if ($k === null) $this->d[] = $v; else $this->d[$k] = $v; }
    function offsetUnset($k): void { echo "unset ", var_export($k, true), "\n"; unset($this->d[$k]); }
    function count(): int { return count($this->d); }
}
$a = new A;
$a['x'] = 1; $a[] = 2; $a[5] = 'five';
var_dump($a['x'], isset($a['x']), isset($a['zz']), empty($a['x']), empty($a['zz']), count($a));
$a['x'] += 10; var_dump($a['x']);
$a['x']++; ++$a['x']; var_dump($a['x']);
$a['arr'] = [1, 2];
$a['arr'][] = 3; var_dump($a['arr']);
$a['obj'] = new A; $a['obj']['inner'] = 'in'; var_dump($a['obj']->d);
unset($a['x']); var_dump($a['x'] ?? 'def');
[$p, $q] = $a; var_dump($p, $q);
$wrap = ['a' => $a]; $wrap['a']['w'] = 'via array'; var_dump($a->d['w']);
var_dump(isset($wrap['a']['w']), isset($wrap['a']['nope']));
try { $o = new stdClass; $o['x'] = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $o = new stdClass; var_dump($o['x']); } catch (Error $e) { echo $e->getMessage(), "\n"; }
