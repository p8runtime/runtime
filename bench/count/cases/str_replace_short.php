<?php
// One str_replace of a one-character search in a short subject.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $t = 0; $s = "a quick brown fox"; for ($i = 0; $i < $n; $i++) { $t += strlen(str_replace("q", "kw", $s)); } return $t; }
echo run($n), "\n";
