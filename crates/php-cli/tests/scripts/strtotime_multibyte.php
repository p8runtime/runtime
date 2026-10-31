<?php
// Text that is not a date, multibyte included, is false (never a crash).
foreach (['ö', 'nächste Woche', '1 März 2026', 'tomorrow ö', "\u{1F600}", 'Jänner', '+1 day'] as $t) {
    var_dump(strtotime($t, 1767225600) === false);
}
try { new DateTime('Montag, 5. Jänner'); } catch (Exception $e) { echo get_class($e), "\n"; }
var_dump(date_create('über') === false);
