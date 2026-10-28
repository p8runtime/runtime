<?php
// unset() in a file included at global scope empties the global for every
// file bound to it; a later assignment in any of them is the global again.
$keep = 'k';
require __DIR__ . '/include_unset_global_2.inc';
require __DIR__ . '/include_unset_global_3.inc';
echo "main: action=", $action ?? 'unset', " G=", $GLOBALS['action'] ?? 'unset', " v=", $v ?? 'unset', " b=", $b, " list=", json_encode($list), " defined=", isset($gone) ? 'y' : 'n', "\n";
