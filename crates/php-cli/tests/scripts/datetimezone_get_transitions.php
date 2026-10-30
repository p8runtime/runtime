<?php
// DateTimeZone::getTransitions: the state at begin, then each change up to end.
$z = new DateTimeZone('America/St_Johns');
var_dump($z->getTransitions(1767225600, 1767225600));
print_r((new DateTimeZone('Europe/London'))->getTransitions(1704067200, 1735689599));
var_dump((new DateTimeZone('+02:00'))->getTransitions(0, 0));
var_dump(count((new DateTimeZone('UTC'))->getTransitions(0, 1735689599)));
