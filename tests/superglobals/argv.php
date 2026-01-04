<?php
global $argv;
echo "Argument count: " . count($argv) . "\n";
foreach ($argv as $index => $value) {
    echo "{$index}: " . basename($value) . "\n";
}
