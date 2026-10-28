<?php
var_dump(error_get_last());
$v = @$nope;
var_dump(error_get_last()["message"]);
echo "x";
$r = @hex2bin("abc");
$e = error_get_last();
$e["file"] = basename($e["file"]);
var_dump($r, $e);
error_clear_last();
var_dump(error_get_last());
var_dump(bin2hex(hex2bin("48656c6c6f00ff")), hex2bin("zz"));
$e = error_get_last();
var_dump($e['type'] === E_WARNING, $e['line']);
set_error_handler(fn() => true);
trigger_error("handled", E_USER_NOTICE);
restore_error_handler();
var_dump(error_get_last()['message']);
