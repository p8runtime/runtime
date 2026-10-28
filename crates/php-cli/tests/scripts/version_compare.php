<?php
$versions = ['', '1', '1.0', '1.0.0', '1.0.1', '1.1', '1.10', '1.9', '5.3.0-dev', '5.3.0alpha1', '5.3.0a2', '5.3.0beta1',
    '5.3.0b1', '5.3.0RC1', '5.3.0rc2', '5.3.0', '5.3.0pl1', '5.3.0p1', '5.3.0-foo', '5.3.0#1', '#1', '1..2', '1-_+2',
    '1.0.0.0', '6.8.3', '6.8', '7.2.24', '8.4.27', '1.2.3a', '1.2.3.a', 'abc', '1!2', '99999999999999999999', '1.0RC1', '1.0rc1-dev'];
foreach ($versions as $a) {
    $row = '';
    foreach ($versions as $b) {
        $row .= ['-', '=', '+'][version_compare($a, $b) + 1];
    }
    echo str_pad(var_export($a, true), 24), $row, "\n";
}
foreach (['<', 'lt', '<=', 'le', '>', 'gt', '>=', 'ge', '==', '=', 'eq', '!=', '<>', 'ne', null] as $op) {
    var_dump(version_compare('5.2', '5.10', $op), version_compare('5.10', '5.10', $op));
}
try { version_compare('1', '2', 'bogus'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(version_compare("1.0\0zzz", "1.0"));
var_dump(version_compare(PHP_VERSION, '7.2.24', '>='));
