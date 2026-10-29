<?php
// log_errors with error_log set: each error that error_reporting lets
// through is appended to the file, dated, as "PHP Warning:  …" (whatever
// html_errors says; not when silenced with @); error_log() too. Also
// parse_url('') is an empty path.
$f = __DIR__ . '/log_errors_file/test.log';
@unlink($f);
ini_set('error_log', $f);
ini_set('log_errors', '1');
ini_set('display_errors', '0');
trigger_error('a notice', E_USER_NOTICE);
echo $undefined;
trigger_error('a deprecation', E_USER_DEPRECATED);
@trigger_error('silenced', E_USER_WARNING);
error_log('direct line');
ini_set('html_errors', '1');
trigger_error('html on', E_USER_WARNING);
echo str_replace(__FILE__, 'FILE', preg_replace('/^\[\d\d-[A-Z][a-z]{2}-\d{4} \d\d:\d\d:\d\d UTC\] /m', '[TIME] ', file_get_contents($f)));
unlink($f);
var_dump(parse_url(''), parse_url('', PHP_URL_PATH), parse_url('?'));
