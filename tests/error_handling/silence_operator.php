<?php
$a = @$undef;
var_dump($a);
echo @$undef2, "|\n";
$b = @$arr['k'];
var_dump($b, error_get_last()['message']);
echo $loud, "|\n";
