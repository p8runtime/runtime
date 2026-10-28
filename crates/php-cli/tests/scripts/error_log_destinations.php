<?php
// error_log() to a file (type 3), through the error_log ini (type 0, dated), and its errors.
$dir = sys_get_temp_dir() . '/p8-error-log-' . getmypid();
@mkdir($dir);
$f = "$dir/a.log";
var_dump(error_log("one\n", 3, $f), error_log('two', 3, $f), file_get_contents($f));
ini_set('error_log', "$dir/b.log");
var_dump(error_log('dated'));
var_dump(preg_match('/^\[\d\d-[A-Z][a-z]{2}-\d{4} \d\d:\d\d:\d\d UTC\] dated\n$/', file_get_contents("$dir/b.log")));
var_dump(@error_log('x', 3, "$dir/missing/c.log"));
echo str_replace($dir, 'DIR', error_get_last()['message']), "\n";
try { error_log('x', 2); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
unlink($f); unlink("$dir/b.log"); rmdir($dir);
