<?php
$arr = [1, 2, 3];
array_splice($arr, 1, 1, ['x']);
print_r($arr);
