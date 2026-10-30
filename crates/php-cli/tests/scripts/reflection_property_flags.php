<?php
abstract class A {
    final public int $f = 1;
    public $plain;
    public readonly int $ro;
    public static $st;
    abstract public string $abs { get; }
    public string $virt { get => "v"; }
    public string $backed = "" { set => strtoupper($value); }
}
foreach (['f', 'plain', 'ro', 'st', 'abs', 'virt', 'backed'] as $n) {
    $p = new ReflectionProperty('A', $n);
    echo $n, ': mods=', $p->getModifiers(), ' final=', var_export($p->isFinal(), true), ' abstract=', var_export($p->isAbstract(), true),
        ' virtual=', var_export($p->isVirtual(), true), ' hooks=', var_export($p->hasHooks(), true), ' [', implode(',', array_keys($p->getHooks())), "]\n";
}
$h = (new ReflectionProperty('A', 'virt'))->getHooks()['get'];
echo get_class($h), ' ', $h->getName(), "\n";
