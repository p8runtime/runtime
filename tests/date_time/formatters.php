<?php
$dt = new DateTimeImmutable('2025-01-01 00:00:00', new DateTimeZone('America/New_York'));
echo $dt->format('Y-m-d H:i P') . "\n";
$dt2 = new DateTimeImmutable('2025-01-02 02:00:00', new DateTimeZone('America/New_York'));
echo $dt2->format('Y-m-d H:i P') . "\n";
