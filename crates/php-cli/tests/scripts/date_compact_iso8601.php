<?php
// Compact ISO 8601 (timelib datenocolon / xmlrpc): 20080701, 20080701T22:38:07, 20080701T223807.
foreach (['20261029T23:30:37+0100', '20261029T23:30:37', '20261029T233037', '20261029', '2026-10-29T23:30:37+01:00', '20261029t23:30:37Z', '20080701T22:38:07', '20080701T09:38:07Z'] as $s) {
    $d = date_create($s, new DateTimeZone('UTC')); echo $s, ' => ', $d ? $d->format('Y-m-d H:i:s P') : 'false', "\n";
}
