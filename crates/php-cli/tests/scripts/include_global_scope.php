<?php
// Files included at global scope, two levels deep: their variables are the globals.
$_SERVER['X'] = 'set';
$m = 'main';
require __DIR__ . '/include_global_scope_2.inc';
echo "main: m=$m n2=$n2 n3=$n3 dyn=", $dyn ?? '-', " gone=", isset($gone) ? 'yes' : 'no', "\n";
