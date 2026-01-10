<?php

require_once __DIR__ . '/../php_modules/json.php';

use function json\{ encode, decode, last_error, last_error_msg };

$data = ["name" => "php-rs", "count" => 3];
$json = encode($data);
$decoded = decode($json, true);

echo $json . "\n";
echo $decoded["name"] . "\n";
echo last_error() . "\n";
echo last_error_msg() . "\n";
