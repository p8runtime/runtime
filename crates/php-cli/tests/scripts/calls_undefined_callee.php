<?php
// `$f(...)` with $f undefined: the warning reaches the error handler first,
// then the call fails before its arguments are read (INIT_DYNAMIC_CALL).
set_error_handler(function ($no, $msg) { echo "handler: $msg\n"; return true; });
try { $g($undefinedArg); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
function byRef(&$x) { $x = 5; }
$h = 'byRef';
$h($fresh);
var_dump($fresh);
set_error_handler(function ($no, $msg) { throw new Exception("from handler: $msg"); });
try { $k($fresh2); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump(isset($fresh2));
