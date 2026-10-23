<?php
class D { function __construct(public $n) {} function __destruct() { echo "bye {$this->n}\n"; } }
$a = new D('a'); $b = new D('b'); $c = new D('c'); $c2 = $c;
$arr = [new D('in-array')];
class S { static $k; } S::$k = new D('static');
$x = new D('x'); $x = null; echo "after x\n";
$y = new D('y'); $y = new D('y2'); echo "after y\n";
function f() { $l = new D('local'); echo "in f\n"; } f(); echo "after f\n";
new D('temp'); echo "after temp\n";
$cy = new D('cycle'); $cy->self = $cy; unset($cy); echo "after cycle\n";
try { $t = new class { function __construct() { throw new Exception('ctor'); } function __destruct() { echo "never\n"; } }; } catch (Exception $e) { echo "caught\n"; }
register_shutdown_function(function () { echo "shutdown\n"; });
echo "end\n";
