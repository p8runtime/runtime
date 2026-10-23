<?php
class D { function __construct(public $n, public $child = null) {} function __destruct() { echo "bye {$this->n}\n"; } function m() { echo "m {$this->n}\n"; return $this->n; } }
class T { function __destruct() { throw new Exception('from dtor'); } }
$a = new D('a'); unset($a); echo "after unset\n";
$arr = [new D('e0'), new D('e1')]; unset($arr[0]); echo "after unset elem\n"; $arr = []; echo "after arr\n";
$p = new D('parent', new D('kid')); $p = null; echo "after nested\n";
(new D('tmp'))->m(); echo "after tmp call\n";
echo (new D('expr'))->m(), "\n";
try { $t = new T; unset($t); echo "not here\n"; } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
function g() { return new D('ret'); } g(); echo "after g\n"; $k = g(); echo "kept\n";
$o = new D('prop-holder'); $o->child = new D('in-prop'); $o->child = null; echo "after prop null\n";
foreach ([new D('loop1'), new D('loop2')] as $x) { echo "iter {$x->n}\n"; } echo "after loop\n"; unset($x);
$gen = (function () { $l = new D('gen-local'); yield 1; yield 2; })(); foreach ($gen as $v) { echo "gen $v\n"; } echo "after gen\n"; unset($gen); echo "gen gone\n";
$s = new D('survivor');
exit("exiting\n");
