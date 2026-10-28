<?php
// The procedural forms of DateTimeImmutable::createFromFormat(),
// DateTime::setISODate(), DateTimeZone::getOffset() and
// DateInterval::createFromDateString().
$d = date_create_immutable_from_format('Y-m-d H:i:s', '2024-02-03 04:05:06', new DateTimeZone('UTC'));
var_dump(get_class($d), $d->format(DATE_ATOM));
$m = date_create('2024-01-01'); date_isodate_set($m, 2024, 10, 3); echo $m->format('Y-m-d'), "\n";
var_dump(timezone_offset_get(new DateTimeZone('Europe/Paris'), new DateTime('2024-07-01')));
var_dump(timezone_offset_get(new DateTimeZone('America/New_York'), new DateTime('2024-01-15')));
var_dump(date_interval_create_from_date_string('3 days')->d, date_interval_create_from_date_string('2 hours')->h);
