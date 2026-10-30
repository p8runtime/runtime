<?php
// The php, php_binary and php_serialize serializers: what session_encode()
// writes, what session_decode() reads, and what it refuses.
ob_start();
ini_set('session.gc_probability', '0');
$dir = sys_get_temp_dir() . '/p8_session_ser_' . getmypid();
mkdir($dir);
session_save_path($dir);

class Point
{
    public $x = 1;
    protected $y = 2;
    private $z = 3;
}

$values = ['int' => 42, 'float' => 1.5, 'str' => "a|b\0c", 'bool' => false, 'null' => null, 'list' => [1, [2, 3]], 'obj' => new Point()];

foreach (['php', 'php_binary', 'php_serialize'] as $handler) {
    $sid = 'p8ser-' . str_replace('_', '', $handler);
    echo "== $handler\n";
    var_dump(ini_set('session.serialize_handler', $handler) !== false);
    session_id($sid);
    session_start();
    $_SESSION = $values;
    $encoded = session_encode();
    echo addcslashes($encoded, "\0..\37"), "\n";
    session_write_close();
    var_dump(file_get_contents("$dir/sess_$sid") === $encoded);
    unset($_SESSION);
    session_start();
    var_dump($_SESSION == $values, array_keys($_SESSION));
    var_dump(session_decode($encoded));
    var_dump($_SESSION == $values);
    session_abort();
}

echo "== a name with the separator, a number as a name\n";
ini_set('session.serialize_handler', 'php');
session_id('p8ser-sep');
session_start();
$_SESSION = ['a|b' => 1, 'ok' => 2];
var_dump(session_encode());
$_SESSION = [5 => 'five', 'ok' => 2];
var_dump(session_encode());
$_SESSION = [];
var_dump(session_encode());
session_abort();

echo "== decoding bad data\n";
foreach (['php', 'php_binary', 'php_serialize'] as $handler) {
    ini_set('session.serialize_handler', $handler);
    session_id('p8ser-bad');
    session_start();
    $_SESSION = ['keep' => 1];
    foreach (["x", "x|", "x|i:1", "x|i:1;y|", "\x05abc", "garbage"] as $data) {
        $r = @session_decode($data);
        echo $handler, ' ', addcslashes($data, "\0..\37"), ' => ', var_export($r, true), ' ', json_encode($_SESSION), ' ', session_status(), "\n";
        if (session_status() !== PHP_SESSION_ACTIVE) {
            session_start();
            $_SESSION = ['keep' => 1];
        }
    }
    session_abort();
}

echo "== an unknown handler\n";
var_dump(ini_set('session.serialize_handler', 'nope'));
var_dump(ini_get('session.serialize_handler'));

foreach (glob("$dir/sess_*") as $f) {
    unlink($f);
}
rmdir($dir);
