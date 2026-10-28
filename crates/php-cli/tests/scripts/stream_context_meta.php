<?php
$c = stream_context_create(['http' => ['method' => 'POST']], ['notification' => 'cb']);
var_dump(get_resource_type($c));
var_dump(stream_context_set_option($c, 'http', 'timeout', 5));
var_dump(stream_context_get_options($c));
var_dump(stream_context_set_params($c, ['options' => ['ftp' => ['overwrite' => true]]]));
var_dump(stream_context_get_params($c));
$d = stream_context_get_default(['http' => ['user_agent' => 'p8']]);
var_dump($d === stream_context_get_default(), stream_context_get_options($d));
try { stream_context_create(['http' => 1]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(get_cfg_var('no.such.setting'));

$m = fopen('php://memory', 'w+');
fwrite($m, "line one\nline two\n");
rewind($m);
fgets($m);
$meta = stream_get_meta_data($m);
var_dump($meta['wrapper_type'], $meta['stream_type'], $meta['mode'], $meta['eof'], $meta['uri']);
$name = __DIR__ . '/stream_context_meta.tmp';
file_put_contents($name, 'x');
$f = fopen($name, 'rb+', false, $c);
$meta = stream_get_meta_data($f);
var_dump($meta['wrapper_type'], $meta['stream_type'], $meta['mode'], $meta['seekable']);
fclose($f);
unlink($name);
