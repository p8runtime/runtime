<?php
// defined("A::B"): a class constant the running code can see (leading \ and
// any class-name case allowed); the class is autoloaded, nothing is raised.
namespace N { class B { const W = 2; } }
namespace {
class A { const V = 1; private const P = 3; static function inside() { return defined('A::P'); } }
spl_autoload_register(function ($c) { echo "autoload $c\n"; if ($c === 'Lazy') { eval('class Lazy { const L = 1; }'); } });
var_dump(defined("A::V"), defined("\\A::V"), defined("a::V"), defined("A::v"));
var_dump(defined("\\N\\B::W"), defined("N\\B::W"), defined("\\A::X"), defined("\\PHP_VERSION"));
var_dump(defined("A::P"), A::inside());
var_dump(defined("Nope\\C::V"), defined("Lazy::L"));
}
