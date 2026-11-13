<?php
// A class calling its own private method on an object of a subclass that
// declares a public method of that name calls the private one: its
// parameters decide how the arguments are passed, the first time through a
// call site as every other time.
class P {
    private function f($x) { $x = "P::f wrote"; return "P::f"; }
    private function g(&$x) { $x = "P::g wrote"; return "P::g"; }
    function plain($o) {
        $out = [];
        $v = "v"; $out[] = $o->f($v) . " v=$v";
        $v = "v"; $out[] = $o->g($v) . " v=$v";
        return implode(", ", $out);
    }
    function computed($o) {
        $out = [];
        $v = "v"; $out[] = $o->f($v, strlen("x")) . " v=$v";
        $v = "v"; $out[] = $o->g($v, strlen("x")) . " v=$v";
        return implode(", ", $out);
    }
    function element($o) {
        $out = [];
        $a = ["k" => "v"]; $out[] = $o->f($a["k"]) . " v=" . $a["k"];
        $a = ["k" => "v"]; $out[] = $o->g($a["k"]) . " v=" . $a["k"];
        return implode(", ", $out);
    }
}
class Q extends P {
    public function f(&$x) { $x = "Q::f wrote"; return "Q::f"; }
    public function g($x) { $x = "Q::g wrote"; return "Q::g"; }
}
function outside($o) {
    $out = [];
    $v = "v"; $out[] = $o->f($v) . " v=$v";
    $v = "v"; $out[] = $o->g($v) . " v=$v";
    return implode(", ", $out);
}
$q = new Q;
// The first time through each site, then the second.
echo $q->plain($q), "\n", $q->plain($q), "\n";
echo $q->computed($q), "\n", $q->computed($q), "\n";
echo $q->element($q), "\n", $q->element($q), "\n";
// From outside the class the public ones are called.
echo outside($q), "\n", outside($q), "\n";

// One site, reached from two scopes: a closure bound to each.
$call = function ($o) { $v = "v"; return $o->f($v) . " v=$v"; };
$inP = Closure::bind($call, null, P::class);
$inQ = Closure::bind($call, null, Q::class);
for ($i = 0; $i < 2; $i++) {
    echo $inP($q), " | ", $inQ($q), " | ", $call($q), "\n";
}
