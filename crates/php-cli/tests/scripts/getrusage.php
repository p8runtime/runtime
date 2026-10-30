<?php
// getrusage(): PHP's keys in its order, integers (values vary run to run).
$r = getrusage(); var_dump(array_keys($r), array_sum(array_map('is_int', $r)) === 17, $r['ru_utime.tv_sec'] >= 0, $r['ru_maxrss'] > 0, count(getrusage(1)));
