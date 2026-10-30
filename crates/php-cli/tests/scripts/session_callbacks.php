<?php
// session_set_save_handler() with callables: the deprecation, the checks on
// the arguments, what the callbacks may return, and the optional ones.
ob_start();
ini_set('session.gc_probability', '0');
ini_set('session.cache_limiter', '');

$store = [];
function s_open($path, $name) { echo "open\n"; return true; }
function s_close() { echo "close\n"; return true; }
function s_read($id) { global $store; echo "read($id)\n"; return $store[$id] ?? ''; }
function s_write($id, $data) { global $store; echo "write($id, $data)\n"; $store[$id] = $data; return true; }
function s_destroy($id) { global $store; echo "destroy($id)\n"; unset($store[$id]); return true; }
function s_gc($max) { echo "gc($max)\n"; return 7; }
function s_create() { echo "create\n"; return 'made-by-callback'; }

echo "== argument checks\n";
foreach ([[], ['s_open'], ['s_open', 's_close', 's_read'], ['s_open', 'nope', 's_read', 's_write', 's_destroy', 's_gc'], ['s_open', 's_close', 's_read', 's_write', 's_destroy', 's_gc', 'nope'], [new stdClass()], [new stdClass(), true]] as $args) {
    try {
        var_dump(@session_set_save_handler(...$args));
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}

echo "== the callbacks\n";
var_dump(session_set_save_handler('s_open', 's_close', 's_read', 's_write', 's_destroy', 's_gc'));
var_dump(session_module_name());
session_id('cb1');
var_dump(session_start());
$_SESSION['v'] = 1;
var_dump(session_gc());
session_write_close();

echo "== create_sid\n";
session_set_save_handler('s_open', 's_close', 's_read', 's_write', 's_destroy', 's_gc', 's_create');
session_id('');
session_start();
var_dump(session_id());
session_destroy();
var_dump(strlen(session_create_id()));

echo "== closures and methods\n";
class Callbacks
{
    public static function open($p, $n) { echo "static open\n"; return true; }
    public function close() { echo "method close\n"; return true; }
}
$c = new Callbacks();
session_set_save_handler(['Callbacks', 'open'], [$c, 'close'], fn($id) => '', function ($id, $data) { echo "closure write\n"; return true; }, 's_destroy', 's_gc');
session_id('cb2');
session_start();
$_SESSION['a'] = 1;
session_write_close();

echo "== return values\n";
foreach ([null, 1, 0, -1, 'x', []] as $ret) {
    session_set_save_handler('s_open', 's_close', 's_read', fn($id, $data) => $ret, 's_destroy', 's_gc');
    session_id('cb3');
    session_start();
    $_SESSION['a'] = 1;
    try {
        echo json_encode($ret), ': ', var_export(@session_write_close(), true), "\n";
    } catch (Throwable $e) {
        echo json_encode($ret), ': ', get_class($e), ': ', $e->getMessage(), "\n";
        session_abort();
    }
}

echo "== an exception from a callback\n";
session_set_save_handler('s_open', 's_close', function ($id) { throw new RuntimeException('no read'); }, 's_write', 's_destroy', 's_gc');
session_id('cb4');
try {
    session_start();
} catch (RuntimeException $e) {
    echo $e->getMessage(), "\n";
}
var_dump(session_status() === PHP_SESSION_ACTIVE);
