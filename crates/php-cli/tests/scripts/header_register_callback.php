<?php
// header_register_callback: the callback runs once, just before the headers
// go out (the first unbuffered output, a builtin's too); a later one replaces it.
var_dump(header_register_callback(function () { echo "[first]\n"; }));
var_dump(header_register_callback(function () { echo "[callback ran]\n"; }));
ob_start();
echo "buffered\n";
$b = ob_get_clean();
echo "before echo\n";
echo $b;
echo "after\n";
try { header_register_callback("nope_not_a_fn"); } catch (TypeError $e) { echo $e->getMessage(), "
"; }
