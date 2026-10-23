<?php
class A { public readonly int $x; public function __construct() { $this->x = 1; } }
$a = new A; echo $a->x, "\n";
$a->x = 2;
