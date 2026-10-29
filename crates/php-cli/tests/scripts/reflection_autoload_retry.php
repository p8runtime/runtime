<?php
// A native method (ReflectionClass) asking for a class an autoloader declares:
// the call runs again after the autoloaders, with its arguments.
spl_autoload_register(function ($c) { echo "load $c\n"; if ($c === 'A3') { eval('class A3 {}'); } if ($c === 'A4') { class A4 {} } });
$r = new ReflectionClass('A3'); var_dump($r->getName());
var_dump(class_exists('A4'));
$r = new ReflectionClass('A4'); var_dump($r->getName());
spl_autoload_register(function ($c) { if ($c === 'A5') { class A5 {} } });
$r = new ReflectionClass('A5'); var_dump($r->getName());
