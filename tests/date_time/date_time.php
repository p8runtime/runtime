<?php
// https://www.php.net/manual/en/datetime.format.php
$now = new DateTime("2025-07-04 12:00:00", new DateTimeZone("UTC"));
echo "ISO: " . $now->format(DateTime::ATOM) . "\n";
echo "Custom: " . $now->format("l, F j, Y g:i A") . "\n";
