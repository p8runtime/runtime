<?php
// ReflectionClassConstant::getValue / getConstant(s) run a constant's code (::class,
// global and class constants) as C::NAME does, not return null before it ran.
namespace N;
define('GLOBAL_V', 11);
class Other { const NAMES = [0 => 'Main']; }
class S {
    public const A = ['default' => ['rss' => \Not\Loaded\Feed::class, 'x' => Other::class], 'type' => 'map'];
    public const B = ['default' => GLOBAL_V];
    public const C = ['default' => Other::NAMES, 'type' => 'map'];
    public const D = ['default' => \PHP_VERSION_ID > 0, 'n' => \E_ALL];
}
$c = new \ReflectionClass(S::class);
foreach ($c->getReflectionConstants() as $k) { var_dump($k->getName(), $k->getValue()); }
var_dump(S::A === $c->getConstant('A'));
