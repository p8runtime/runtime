<?php
echo "n3: SERVER=", gettype($_SERVER), " X=", $_SERVER['X'], " m=$m n2=$n2\n";
$n3 = 'three';
$name = 'dyn'; $$name = 'dynamic';
echo "n3 \$\$: ", ${'m'}, " ", ${'n2'}, "\n";
extract(['ext' => 'extracted']);
echo "compact: ", json_encode(compact('m', 'n2', 'ext')), "\n";
$vars = get_defined_vars(); echo "defined has: ", implode(',', array_intersect(['m', 'n2', 'n3', 'ext', '_SERVER', 'dyn'], array_keys($vars))), "\n";
unset($gone);
function f() { global $ext; return $ext; }
echo "global ext: ", f(), "\n";
