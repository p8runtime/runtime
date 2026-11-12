<?php
// A spread array whose element goes to a by-reference parameter: PHP sends
// a reference to the array's own element, so what the method writes shows
// in the array afterwards (by position and by name).
class N { function m($a, &$b, $c = 'c') { $b .= "[$a$c]"; return "$a$b$c"; } }
function byref(&$x) { $x .= '!'; return $x; }
function spread() {
    $o = new N;
    $list = ['p', 'q', 'r'];
    $r = $o->m(...$list);
    $pair = ['b' => 'k', 'a' => 'j'];
    $s = $o->m(...$pair);
    $one = ['f'];
    $t = byref(...$one);
    return "$r $s $t " . implode(',', $list) . ' ' . implode(',', $pair) . ' ' . implode(',', $one);
}
for ($i = 0; $i < 2; $i++) {
    echo spread(), "\n";
}
