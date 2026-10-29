<?php
// In a namespace, f(...$a) falls back to the global f as any call does; global $x makes a missing global a null one (no warning when read).
namespace A\B;
function own(...$a) { return 'own' . count($a); }
$args = [[1], [2]];
var_dump(array_merge(...$args), own(...$args), \strlen(...['abc']), strlen(...['ab']));
class C { function m() { global $nothere, $there; var_dump($nothere, $there, isset($nothere)); switch ($nothere) { default: echo "switched\n"; } } }
$there = 5;
(new C)->m();
function g() { global $later; return $later; }
var_dump(g(), array_key_exists('later', $GLOBALS), $GLOBALS['later']);
