<?php
// What a script may do with the files it is given: read the root, write
// only its temporary directory.
echo 'read: ', substr(file_get_contents(__FILE__), 0, 5), "\n";

$wrote = @file_put_contents(__DIR__ . '/written.txt', 'x');
echo 'write root: ', var_export($wrote, true), ' (', error_get_last()['message'] ?? '', ")\n";

$tmp = sys_get_temp_dir();
$file = $tmp . '/p8-files-test.txt';
echo 'write tmp: ', var_export(file_put_contents($file, 'ok'), true), ' in ', $tmp, "\n";
unlink($file);

echo 'props: ', json_encode($props), "\n";
