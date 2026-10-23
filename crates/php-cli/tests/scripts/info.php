<?php
chdir(__DIR__); @mkdir('info_tmp'); file_put_contents('info_tmp/f.txt', 'hello');
$s = stat('info_tmp/f.txt'); echo count($s), " ", $s['size'], " ", $s[7], " ", array_keys($s)[13], "\n";
var_dump(@symlink('f.txt', 'info_tmp/l'), readlink('info_tmp/l'), filetype('info_tmp/l'), filetype('info_tmp/f.txt'), lstat('info_tmp/l')['size'] > 0, is_link('info_tmp/l'));
$h = fopen('info_tmp/f.txt', 'r'); echo fstat($h)['size'], "\n"; fclose($h);
var_dump(@stat('info_tmp/nope'));
unlink('info_tmp/l'); unlink('info_tmp/f.txt'); rmdir('info_tmp');
function a($x) { b($x, 'two'); }
function b($y, $z) { print_r(array_map(fn($f) => $f['function'] . ':' . $f['line'], debug_backtrace())); debug_print_backtrace(); }
a(1);
class K { function m() { debug_print_backtrace(); } static function s() { (new K)->m(); } }
K::s();
echo count(get_included_files()), basename(get_included_files()[0]), "\n";
vfprintf(STDOUT, "%s-%05.1f\n", ['x', 3.14159]);
