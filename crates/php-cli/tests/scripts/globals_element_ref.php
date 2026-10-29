<?php
// A reference to $GLOBALS['x'] in a function is the global $x itself:
// `$b = &$GLOBALS[...]` (WordPress's get_bookmark), a by-reference
// argument, and a reference into its elements.
$link = new stdClass; $link->id = 1;
function f() { if (isset($GLOBALS['link'])) { $b = & $GLOBALS['link']; $b->id = 2; } return $b; }
var_dump(f()->id, $link->id);
function g() { $r = & $GLOBALS['made']; $r = 'new global'; }
g(); var_dump($made);
function h() { $GLOBALS['arr']['k'][] = 1; $x = & $GLOBALS['arr']['k']; $x[] = 2; }
h(); var_dump($arr);
function k() { $o = & $GLOBALS['link']; $o = 'replaced'; }
k(); var_dump($link);
function m(array &$a) { $a[] = 'm'; } function n() { m($GLOBALS['list']); } $list = []; n(); var_dump($list);
