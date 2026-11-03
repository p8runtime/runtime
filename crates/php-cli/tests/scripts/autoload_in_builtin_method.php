<?php
spl_autoload_register(function ($c) {
    echo "autoload $c\n";
    if ($c === 'UF') { eval('#[Attribute] class UF { function __construct(public int $n) { echo "UF($n)\n"; } }'); }
    if ($c === 'Lazy') { eval('enum Lazy { case A; }'); }
});
#[UF(60)] class Ev {}
$r = new ReflectionClass('Ev');
$attributes = $r->getAttributes();
var_dump($attributes[0]->newInstance()->n);
var_dump((new ReflectionClass('Ev'))->getAttributes()[0]->newInstance()->n);
function g($x = Lazy::A, $y = Missing::B) {}
var_dump((new ReflectionParameter('g', 0))->getDefaultValue());
try { (new ReflectionParameter('g', 1))->getDefaultValue(); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { var_dump((new ReflectionClass('Nope'))->getName()); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
