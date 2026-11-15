<?php
// One preg_replace of a template tag pattern, with a group reference, in a short string with two tags.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $t = 0; $s = "Hello {{ name }}, you have {{ count }} items"; for ($i = 0; $i < $n; $i++) { $t += strlen(preg_replace('/\{\{\s*(\w+)\s*\}\}/', '<$1>', $s)); } return $t; }
echo run($n), "\n";
