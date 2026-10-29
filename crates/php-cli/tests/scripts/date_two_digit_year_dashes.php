<?php
// yy-mm-dd with dashes (timelib iso8601date2) vs dd-mm-yyyy; and dynamic
// properties on subclasses of stdClass (#[AllowDynamicProperties] inherited).
class M extends stdClass {} class M2 extends M {}
$m = new M2; $m->x = 1; var_dump($m);
foreach (['16-03-02 19:13', '16-03-02', '99-12-31 23:59:59', '70-01-01', '69-01-01', '2016-03-02 19:13', '03-02-16'] as $s) {
    $t = strtotime($s . ' UTC'); echo $s, ' => ', $t === false ? 'false' : gmdate('c', $t), "\n";
    $d = date_create($s, new DateTimeZone('UTC')); echo '   ', $d ? $d->format('c') : 'false', "\n";
}
