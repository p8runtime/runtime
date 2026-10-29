<?php
// Runs in the including function's scope: what it binds, the function sees.
global $g;
$alias = &$target;
$target = 'through the reference';
unset($gone);
$made = 'made here';
include __DIR__ . '/nested.inc';
