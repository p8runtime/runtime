<?php
// References in the argument list reach parameters taken by reference:
// call_user_func_array() and unpacking, as WordPress's block visitors do.
function inject(&$b) { $b['attrs']['theme'] = 'tt5'; }
$visit = function (&$block, &$parent = null, $prev = null) { inject($block); return ''; };
$block = ['attrs' => []];
$parent = null;
call_user_func_array($visit, array(&$block, &$parent, null));
var_dump($block);
$b2 = ['attrs' => []];
call_user_func_array('inject', [&$b2]);
var_dump($b2);
$b3 = ['attrs' => []];
$args = [&$b3];
$visit(...$args);
var_dump($b3);
// A by-value parameter still gets a copy.
$keep = 'same';
call_user_func_array(function ($x) { $x = 'changed'; }, [&$keep]);
var_dump($keep, strlen(...[&$keep]));
