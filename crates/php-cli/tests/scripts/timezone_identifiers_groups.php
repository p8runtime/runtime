<?php
// Only canonical zones (no "backward" aliases), by group.
$all = timezone_identifiers_list();
var_dump(count($all), $all[0], end($all), in_array('America/Buenos_Aires', $all), in_array('Africa/Asmera', $all));
var_dump(DateTimeZone::listIdentifiers(DateTimeZone::UTC));
var_dump(count(DateTimeZone::listIdentifiers(DateTimeZone::EUROPE)), DateTimeZone::listIdentifiers(DateTimeZone::ARCTIC));
var_dump(count(DateTimeZone::listIdentifiers(DateTimeZone::AFRICA | DateTimeZone::INDIAN)));
var_dump(md5(implode(',', $all)));
