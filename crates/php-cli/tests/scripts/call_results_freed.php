<?php
// A call's result passed straight on (to another call, an operator, an
// assignment) is freed as soon as it is used: no copy of it lingers in a
// temporary. Closures, methods, static methods, nullsafe calls, callable
// strings and arrays, each used as an argument (#285).
class D {
    function __construct(public $n) {}
    function __destruct() { echo "free {$this->n}\n"; }
}
class M {
    function get($n) { return [new D($n)]; }
    static function sget($n) { return [new D($n)]; }
    function __invoke($n) { return [new D($n)]; }
}
function plain($n) { return [new D($n)]; }
$m = new M;
$a = [new D('captured')];
$f = fn() => $a;
unset($a);
var_dump(count($f()));
$f = null;
echo "closure gone\n";
$n = count($m->get('method'));
echo "after method\n";
$n = count(M::sget('static'));
echo "after static\n";
$n = count($m?->get('nullsafe'));
echo "after nullsafe\n";
$n = count($m('invoke'));
echo "after invoke\n";
$g = 'plain';
$n = count($g('string callable'));
echo "after string callable\n";
$h = [$m, 'get'];
$n = count($h('array callable'));
echo "after array callable\n";
$k = 'sget';
$n = count(M::$k('static by name'));
echo "after static by name\n";
$n = count(plain('named function'));
echo "after named\n";
echo strlen(json_encode($m->get('in json'))), "\n";
echo "after json\n";
$x = count($m->get('nested a')) + count(M::sget('nested b'));
echo "after sum\n";
$s = (fn($v) => $v)($m->get('arg of closure'));
echo "kept by \$s\n";
$s = null;
echo "end\n";
