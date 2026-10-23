<?php
class A { public $a = 1; }
$a = new A;
$a->b = 2;
echo $a->c;
echo "\n", count((array)$a), "\n";
