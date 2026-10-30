<?php
namespace App {
    use Lazy\Dir;
    use LazyGlobal;
    function f($d = Dir::Up, $e = LazyGlobal::A, $n = Dir::class) {}
}
namespace {
    spl_autoload_register(function ($c) {
        echo "autoload $c\n";
        if ($c === 'Lazy\Dir') { eval('namespace Lazy; enum Dir { case Up; }'); }
        if ($c === 'LazyGlobal') { eval('enum LazyGlobal { case A; }'); }
    });
    foreach ((new ReflectionFunction('App\f'))->getParameters() as $p) {
        var_dump($p->getDefaultValue());
    }
    function g($x = Missing::A) {}
    $p = new ReflectionParameter('g', 0);
    try { $p->getDefaultValue(); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
