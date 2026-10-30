<?php
// $_SESSION is the session's own array: the same from every scope, and
// what is written is what it holds when the session is written.
ob_start();
ini_set('session.gc_probability', '0');
ini_set('session.cache_limiter', '');
$dir = sys_get_temp_dir() . '/p8_session_sg_' . getmypid();
mkdir($dir);
session_save_path($dir);

function start()
{
    session_id('p8sg1');
    session_start();
    $_SESSION['from_function'] = true;
}
function show()
{
    var_dump($_SESSION);
}
function add($k, $v)
{
    $_SESSION[$k] = $v;
}
function waiting()
{
    // $_SESSION here is read before the session starts
    return isset($_SESSION) ? 'set' : 'not set';
}

var_dump(isset($_SESSION), waiting());
start();
var_dump(waiting());
add('late', 1);
show();
$alias = &$_SESSION;
$alias['via_reference'] = 2;
var_dump(array_keys($_SESSION));
session_write_close();
var_dump(file_get_contents("$dir/sess_p8sg1"));
var_dump(array_keys($_SESSION));

echo "== replacing the array\n";
session_start();
$_SESSION = ['replaced' => true];
session_write_close();
var_dump(file_get_contents("$dir/sess_p8sg1"));

echo "== no longer an array\n";
session_start();
$_SESSION = 'text';
var_dump(session_encode());
session_write_close();
var_dump(file_get_contents("$dir/sess_p8sg1"));

echo "== unset\n";
session_start();
$_SESSION['x'] = 1;
unset($_SESSION);
var_dump(session_write_close());
var_dump(file_get_contents("$dir/sess_p8sg1"));
var_dump(isset($_SESSION));
session_start();
var_dump($_SESSION);
session_abort();

echo "== numeric keys are skipped\n";
session_start();
$_SESSION = ['a' => 1, 5 => 2];
var_dump(@session_encode());
session_abort();

echo "== objects\n";
class User
{
    public function __construct(public string $name, public array $roles = []) {}
    public function __sleep() { echo "sleep\n"; return ['name']; }
    public function __wakeup() { echo "wakeup\n"; $this->roles = ['restored']; }
}
session_start();
$_SESSION = ['user' => new User('ann', ['admin'])];
session_write_close();
var_dump(file_get_contents("$dir/sess_p8sg1"));
session_start();
var_dump($_SESSION['user']);
session_abort();

foreach (glob("$dir/sess_*") as $f) {
    unlink($f);
}
rmdir($dir);
