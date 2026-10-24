<?php
$a = 1;
function f() { echo $GLOBALS["a"], "\n"; $GLOBALS["b"] = 2; $GLOBALS["arr"]["k"] = 3; unset($GLOBALS["a"]); var_dump(isset($GLOBALS["b"]), isset($GLOBALS["nope"]), $GLOBALS["nope"] ?? "d"); echo $GLOBALS["nope"]; }
f();
var_dump(isset($a), $b, $arr);
$g = $GLOBALS; var_dump(is_array($g), isset($g["b"]), array_key_exists("GLOBALS", $g));
function h() { return count($GLOBALS) > 0; } var_dump(h());
