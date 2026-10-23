<?php
$a = [];
var_dump(@$a['missing'], @$undefined);
echo @$a['x'] . "|\n";
var_dump(error_reporting());
var_dump(@error_reporting());
echo $a['shown'];
function noisy() { echo $GLOBALS_nope; return 1; }
var_dump(@noisy());
var_dump(error_reporting());
