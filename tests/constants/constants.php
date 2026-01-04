<?php
define('GILD_APP', 'gild');
const GILD_VERSION = '0.1';

if (!defined('GILD_APP')) {
    echo "missing constant\n";
}

echo GILD_APP . " " . GILD_VERSION . "\n";
