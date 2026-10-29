<?php
// isset/empty/?? on a string offset, and reading one, with each kind of key
// (WordPress's wptexturize() decides a tag is an opening one by isset($t[1])).
$s = '<script>';
var_dump(isset($s[1]), isset($s[7]), isset($s[8]), isset($s[-1]), isset($s[-9]), isset($s[1][0]), empty($s[1]), empty($s[20]));
function opening($t) { return isset($t[1]) && '/' !== $t[1]; }
var_dump(opening('<p>'), opening('</p>'), opening('<'));
$a = ['k' => 'str'];
$o = new stdClass;
$o->p = 'xyz';
var_dump(isset($a['k'][1]), isset($a['k'][5]), $a['k'][2] ?? 'd', isset($o->p[1]), isset($o->p[3]));
$t = 'abc';
foreach ([1, '1', ' 1', '1 ', '01', '-1', '1.0', 'x', '', 1.0, true, false, null, -3, -4, 3] as $k) {
    echo var_export($k, true), ': ', var_export(isset($t[$k]), true), ' ', var_export(empty($t[$k]), true), ' ', var_export($t[$k] ?? 'D', true), "\n";
}
foreach (['x', '0x1', '1.0', ' 1', '', 1.7, true, null, '2', []] as $k) {
    try {
        echo var_export($k, true), ' read: ', var_export($t[$k], true), "\n";
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
}
var_dump(in_array("file", stream_get_wrappers()), in_array("php", stream_get_wrappers()), in_array("nope", stream_get_wrappers()));
