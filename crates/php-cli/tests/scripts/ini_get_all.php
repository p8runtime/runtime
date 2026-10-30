<?php
$all = ini_get_all(null, true);
var_dump(is_array($all), count($all) > 50);
var_dump($all['memory_limit']['access'], $all['allow_url_fopen']['access'], $all['display_errors']['access']);
var_dump(array_keys($all['precision']));
ini_set('precision', '10');
$p = ini_get_all(null, true)['precision']; var_dump($p['global_value'], $p['local_value']);
var_dump(ini_get_all(null, false)['precision']);
$keys = array_keys($all); $sorted = $keys; sort($sorted); var_dump($keys === $sorted);
$d = ini_get_all('date', false); ksort($d); var_dump(array_keys($d));
var_dump(ini_get_all('ctype'));
var_dump(@ini_get_all('nosuchext'));
