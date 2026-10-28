<?php
require __DIR__ . '/include_unset_global_4.inc';
echo "gu3 after unset: ", $action ?? 'unset', " G=", $GLOBALS['action'] ?? 'unset', " in vars: ", array_key_exists('action', get_defined_vars()) ? 'y' : 'n', "\n";
$action = 'login';
function f() { global $action; return $action; }
echo "gu3: f()=", f(), " G=", $GLOBALS['action'], "\n";
$list = [1, 2]; foreach ($list as &$v) { $v *= 2; } unset($v);
$b = 'b'; $a = &$b; unset($a);
$gone = 1; unset($gone);
