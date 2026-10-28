<?php
var_dump($argc, count($argv), $argv[0] === $_SERVER['argv'][0], $_SERVER['argc']);
var_dump($_GET, $_POST, is_array($_SERVER), isset($_SERVER['REQUEST_TIME']));
function inner() { return [count($_SERVER['argv']), $_GET]; }
var_dump(inner());
$_SERVER['argv'] = ['prog', '-a', '-bvalue', '-c', 'x', '--long=1', '--flag', '--opt', '-v', '-v', 'rest'];
var_dump(getopt('ab:c:v', ['long:', 'flag', 'opt::'], $rest), $rest);
$_SERVER['argv'] = ['prog', '-xy', '--', '-a'];
var_dump(getopt('xya', [], $rest), $rest);
