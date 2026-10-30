<?php
// stream_wrapper_register and friends: what fails and how PHP says so.
class W {
    public $context;
    function stream_open($path, $mode, $options, &$opened) { echo "open $path $mode $options\n"; return true; }
    function stream_read($n) { return ''; }
    function stream_eof() { return true; }
    function stream_stat() { return []; }
    function url_stat($path, $flags) { echo "url_stat $path $flags\n"; return ['size' => 3, 'mode' => 0100644]; }
}

var_dump(stream_wrapper_register('mem', 'W'));
var_dump(stream_wrapper_register('mem', 'W'));
var_dump(stream_wrapper_register('a b', 'W'));
var_dump(stream_wrapper_register('php', 'W'));
var_dump(stream_register_wrapper('mem2', 'W'));
try {
    stream_wrapper_register('x1', 'NoSuchClass');
} catch (TypeError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
var_dump(in_array('mem', stream_get_wrappers()), in_array('mem2', stream_get_wrappers()));

echo "-- unregistered\n";
var_dump(stream_wrapper_unregister('mem'));
var_dump(stream_wrapper_unregister('mem'));
var_dump(in_array('mem', stream_get_wrappers()));
var_dump(file_get_contents('mem://x'));
var_dump(file_exists('mem://x'));
var_dump(file_get_contents('nope://x'));
var_dump(stream_wrapper_restore('mem'));
var_dump(stream_wrapper_restore('php'));

echo "-- case\n";
var_dump(stream_wrapper_register('Abc', 'W'));
var_dump(fopen('abc://q', 'r'));
var_dump(is_resource(fopen('Abc://q', 'r')));
var_dump(stream_wrapper_register('def', 'W'));
var_dump(is_resource(fopen('DEF://q', 'r')));

echo "-- file:// unregistered\n";
var_dump(stream_wrapper_unregister('file'));
var_dump(in_array('file', stream_get_wrappers()));
var_dump(file_get_contents('/nonexistent/p8-wrapper.txt'));
var_dump(fopen('/nonexistent/p8-wrapper.txt', 'r'));
var_dump(file_exists('/nonexistent/p8-wrapper.txt'));
var_dump(stream_wrapper_register('file', 'W'));
var_dump(file_get_contents('rel/name.txt'));
var_dump(file_exists('/some/where'));
var_dump(file_exists('/some/where'));
var_dump(filesize('/some/where'));
clearstatcache();
var_dump(filesize('/some/where'));
var_dump(stream_wrapper_restore('file'));
var_dump(stream_wrapper_restore('file'));
var_dump(file_get_contents('/nonexistent/p8-wrapper.txt'));

echo "-- a class that cannot serve\n";
abstract class Abstracted {}
var_dump(stream_wrapper_register('abs', 'Abstracted'));
var_dump(fopen('abs://q', 'r'));
var_dump(file_exists('abs://q'));
class Bare {}
stream_wrapper_register('bare', 'Bare');
var_dump(fopen('bare://q', 'r'));
var_dump(file_exists('bare://q'));
var_dump(stat('bare://q'));
var_dump(unlink('bare://q'));
var_dump(rename('bare://q', 'bare://r'));
var_dump(rename('bare://q', 'abc://r'));
var_dump(rename('bare://q', '/tmp/r'));
var_dump(mkdir('bare://q'));
var_dump(rmdir('bare://q'));
var_dump(touch('bare://q'));
var_dump(chmod('bare://q', 0644));
var_dump(opendir('bare://q'));
var_dump(scandir('bare://q'));

echo "-- a stream with few methods\n";
class Few {
    public $context;
    function stream_open($path, $mode, $options, &$opened) { return true; }
}
stream_wrapper_register('few', 'Few');
$f = fopen('few://x', 'r+');
var_dump(fread($f, 10));
var_dump(fwrite($f, 'abc'));
var_dump(feof($f));
var_dump(fstat($f));
var_dump(ftruncate($f, 0));
var_dump(flock($f, LOCK_SH));
var_dump(fflush($f));
var_dump(fseek($f, 10));
var_dump(fseek($f, -1, SEEK_CUR));
var_dump(stream_get_meta_data($f)['seekable']);
var_dump(fclose($f));
var_dump(stream_set_blocking(fopen('few://y', 'r'), false));

echo "-- a stream that answers too much\n";
class Greedy {
    public $context;
    function stream_open($path, $mode, $options, &$opened) { return true; }
    function stream_read($n) { return str_repeat('x', $n + 5); }
    function stream_write($d) { return strlen($d) + 2; }
    function stream_eof() { return false; }
    function stream_truncate($n) { return 'yes'; }
    function stream_lock($op) { return 1; }
}
stream_wrapper_register('greedy', 'Greedy');
$f = fopen('greedy://x', 'r+');
var_dump(strlen(fread($f, 3)));
var_dump(fwrite($f, 'abcd'));
var_dump(ftruncate($f, 2));
var_dump(flock($f, LOCK_EX));
try {
    flock($f, 0);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}

echo "-- url wrappers and include\n";
var_dump(stream_wrapper_register('remote', 'W', STREAM_IS_URL));
var_dump(stream_is_local('remote://x'), stream_is_local('mem2://x'));
var_dump(include 'remote://inc.php');
try {
    require 'remote://req.php';
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
var_dump(file_put_contents('mem2://locked', 'x', LOCK_EX));
