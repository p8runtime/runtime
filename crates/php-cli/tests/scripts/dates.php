<?php
$t = 1700000000;
echo date_default_timezone_get(), "\n";
echo date('Y-m-d H:i:s D l N j S w z W F M n t L o y a A g G h B u v e I O P p T Z U', $t), "\n";
echo date('c | r | \Y\e\s', $t), "\n";
echo gmdate('Y-m-d H:i:s T e', $t), "\n";
date_default_timezone_set('America/New_York');
echo date_default_timezone_get(), " ", date('Y-m-d H:i:s T e I O P Z', $t), " ", date('c', 1720000000), " ", date('T I', 1720000000), "\n";
var_dump(date_default_timezone_set('Nowhere/Land'));
echo mktime(12, 0, 0, 1, 15, 2024), " ", mktime(0, 0, 0, 13, 1, 2023), " ", mktime(0, 0, 0, 3, 0, 2024), " ", gmmktime(0, 0, 0, 1, 1, 1970), " ", mktime(25, 70, 70, 2, 30, 24), "\n";
var_dump(checkdate(2, 29, 2024), checkdate(2, 29, 2023), checkdate(13, 1, 2024));
print_r(getdate($t)); print_r(localtime($t, true));
echo idate('Y', $t), idate('m', $t), idate('t', $t), idate('z', $t), "\n";
date_default_timezone_set('UTC');
$base = 1700000000;
foreach (['2024-01-15', '2024-01-15 10:30:00', '2024-01-15T10:30:00Z', '2024-01-15T10:30:00+02:00', '01/15/2024', '15-01-2024', '15.01.2024', 'January 15 2024', '15 January 2024', 'Jan 15, 2024 3:30 pm', '+1 day', '-2 weeks', '+1 week 2 days', '1 year ago', 'tomorrow', 'yesterday', 'today', 'midnight', 'noon', 'now', 'next monday', 'last friday', 'monday', 'first day of next month', 'last day of this month', '@86400', '10:00', '3pm', '2024-02-30', 'garbage', '+1 month', 'last day of february 2024', 'next month', '2024-01-31 +1 month', 'saturday this week', '10 September 2000 +1 week', 'Sep 2021'] as $s) {
    $r = strtotime($s, $base);
    echo str_pad($s, 34), " ", $r === false ? 'false' : date('Y-m-d H:i:s', $r), "\n";
}
