<?php
// A literal where the method takes a reference: PHP finds the method, sends
// the arguments before the literal, and throws at the literal's send
// ("could not be passed by reference"); the method does not run, and a
// variable sent by reference before it stays one the method never wrote.
class T {
    function refval(&$a, $b) { $a .= 'a'; $b .= 'b'; return "$a $b"; }
    function valref($a, &$b) { $a .= 'a'; $b .= 'b'; return "$a $b"; }
    function refref(&$a, &$b) { $a .= 'a'; $b .= 'b'; return "$a $b"; }
}
function literal($o) {
    $v = 'v';
    try {
        $r = $o->valref($v, 'literal');
    } catch (Error $e) {
        $r = get_class($e) . ': ' . $e->getMessage();
    }
    try {
        $s = $o->refval(5, $v);
    } catch (Error $e) {
        $s = get_class($e) . ': ' . $e->getMessage();
    }
    try {
        $t = $o->refref($v, 'second');
    } catch (Error $e) {
        $t = get_class($e) . ': ' . $e->getMessage();
    }
    return "$r | $s | $t | $v";
}
for ($i = 0; $i < 2; $i++) {
    echo literal(new T), "\n";
}
