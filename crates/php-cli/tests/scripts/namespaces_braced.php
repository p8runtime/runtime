<?php
namespace A {
    class Thing { const K = 'a'; }
    function f() { return __FUNCTION__; }
}
namespace B {
    use A\Thing;
    use function A\f;
    use A as Alias;
    echo Thing::K, f(), Alias\Thing::K, "\n";
    echo \A\f(), "\n";
}
namespace {
    echo A\Thing::K, "\n";
    var_dump(new B\Missing);
}
