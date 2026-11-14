<?php
// One nested array write (`$a[x][y] = v`): the write_path walk.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $a = []; for ($i = 0; $i < $n; $i++) { $a[$i & 7]["x"] = $i; } return count($a); }
echo run($n), "\n";
