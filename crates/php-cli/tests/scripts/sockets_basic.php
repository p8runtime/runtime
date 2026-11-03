<?php
var_dump(extension_loaded('sockets'));
$l = socket_create_listen(0);
var_dump($l instanceof Socket);
var_dump(socket_getsockname($l, $addr, $port), $addr, is_int($port) && $port > 0);
$c = socket_create(AF_INET, SOCK_STREAM, SOL_TCP);
var_dump(socket_connect($c, '127.0.0.1', $port));
$a = socket_accept($l);
var_dump(socket_write($c, "hello\nworld"));
var_dump(socket_read($a, 100, PHP_NORMAL_READ));
var_dump(socket_read($a, 5));
var_dump(socket_getpeername($a, $paddr), $paddr);
socket_close($c);
var_dump(socket_read($a, 10));
socket_close($a);
socket_close($l);
try { socket_close($l); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
var_dump(socket_strerror(SOCKET_ECONNREFUSED) !== '');
$s = socket_create(AF_INET, SOCK_STREAM, 0);
var_dump(socket_set_option($s, SOL_SOCKET, SO_REUSEADDR, 1));
var_dump(socket_get_option($s, SOL_SOCKET, SO_TYPE) === SOCK_STREAM);
