<?php
// $GLOBALS and get_defined_vars() list the script's globals in the order
// they were made (the request's own globals are left out here).
$zeta = 1;
$alpha = 2;
$name = 'dyn1'; $$name = 3;
extract(['ext2' => 4, 'ext1' => 5]);
$GLOBALS['via_globals'] = 6;
function make() { $GLOBALS['from_fn'] = 7; global $declared; $declared = 8; }
make();
$own = fn($keys) => array_values(array_filter($keys, fn($k) => $k[0] !== '_' && $k !== 'argv' && $k !== 'argc'));
echo implode(',', $own(array_keys($GLOBALS))), "\n";
echo implode(',', $own(array_keys(get_defined_vars()))), "\n";
unset($alpha);
$alpha = 'again';
echo implode(',', $own(array_keys($GLOBALS))), "\n";
