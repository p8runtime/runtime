<?php
// A file included in a function shares the function's variables, as one
// symbol table: `global`, `=&` and unset() in it rebind the function's
// variables too (PHPUnit includes WordPress's bootstrap from a method, and
// the bootstrap's `global $wpdb` must reach PHPUnit's copy-back).
$g = 'global g'; $h = 'global h'; $t = 'global t';
function f() {
    $gone = 'still here';
    include __DIR__ . '/include_shares_scope/part.inc';
    var_dump($g, $h, $alias, isset($gone), $made, $deep);
    $g = 'changed by f';
    try { include __DIR__ . '/include_shares_scope/throws.inc'; } catch (Exception $e) { echo $e->getMessage(), "\n"; }
    $t = 'changed by f too';
}
f();
var_dump($g, $t);
