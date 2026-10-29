<?php
// Two different closures are two autoloaders (WordPress 7 registers one for
// its AI client after another's); the same callable twice is one.
spl_autoload_register(function ($c) { echo "first: $c\n"; });
spl_autoload_register(static function ($c) { echo "second: $c\n"; if ($c === 'Late') { eval('class Late {}'); } });
var_dump(count(spl_autoload_functions()), class_exists('Late'), class_exists('Missing'));
function loader($c) { echo "named: $c\n"; }
spl_autoload_register('loader');
spl_autoload_register('LOADER');
class L { static function load($c) {} function inst($c) {} }
spl_autoload_register(['L', 'load']);
spl_autoload_register(['l', 'LOAD']);
spl_autoload_register('L::load');
$a = new L;
$b = new L;
spl_autoload_register([$a, 'inst']);
spl_autoload_register([$a, 'inst']);
spl_autoload_register([$b, 'inst']);
var_dump(count(spl_autoload_functions()));
var_dump(spl_autoload_unregister('loader'), count(spl_autoload_functions()));
