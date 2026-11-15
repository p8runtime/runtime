<?php
// One preg_match of a router pattern with three groups against a request path.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $t = 0; $p = "/users/42/posts/126?page=6"; for ($i = 0; $i < $n; $i++) { if (preg_match('#^/users/(\d+)/posts/(\d+)(?:\?page=(\d+))?$#', $p, $m)) { $t += (int) $m[1] + (int) $m[3]; } } return $t; }
echo run($n), "\n";
