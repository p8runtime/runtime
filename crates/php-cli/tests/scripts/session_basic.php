<?php
// The files handler: start, write, read back, the status transitions, the
// settings. Nothing about the save path is printed.
ob_start();
$dir = sys_get_temp_dir() . '/p8_session_basic_' . getmypid();
mkdir($dir);

var_dump(extension_loaded('session'));
var_dump(PHP_SESSION_DISABLED, PHP_SESSION_NONE, PHP_SESSION_ACTIVE);
var_dump(session_status() === PHP_SESSION_NONE);
var_dump(session_id());
var_dump(session_name());
var_dump(session_module_name());
var_dump(ini_get('session.save_handler'), ini_get('session.serialize_handler'));
var_dump(session_save_path(''));
var_dump(session_save_path($dir) === '');
var_dump(session_save_path() === $dir);

var_dump(session_id('p8basic1'));
var_dump(session_start());
var_dump(session_status() === PHP_SESSION_ACTIVE);
var_dump(session_id());
var_dump($_SESSION);
$_SESSION['count'] = 1;
$_SESSION['name'] = 'alice';
$_SESSION['list'] = [1, 2.5, true, null, 'x'];
var_dump(session_encode());
var_dump(session_write_close());
var_dump(session_status() === PHP_SESSION_NONE);
var_dump(file_get_contents("$dir/sess_p8basic1"));
var_dump(count(glob("$dir/sess_*")));

// The data is back, and $_SESSION is the array the session keeps.
unset($_SESSION);
var_dump(session_start());
var_dump($_SESSION);
function bump() {
    $_SESSION['count']++;
}
bump();
var_dump($_SESSION['count']);
var_dump(session_commit());
var_dump(file_get_contents("$dir/sess_p8basic1"));

// session_unset empties it, session_abort leaves the file alone
session_start();
var_dump(session_unset());
var_dump($_SESSION);
var_dump(session_abort());
var_dump(file_get_contents("$dir/sess_p8basic1") !== '');
var_dump(session_abort(), session_unset(), session_write_close());

// session_reset takes the stored data back
session_start();
$_SESSION['changed'] = true;
var_dump(session_reset());
var_dump(isset($_SESSION['changed']), $_SESSION['name']);
session_write_close();

// session_destroy removes the file and keeps $_SESSION
session_start();
var_dump(session_destroy());
var_dump(file_exists("$dir/sess_p8basic1"));
var_dump(session_status() === PHP_SESSION_NONE);
var_dump(session_id());
var_dump(isset($_SESSION['name']));
var_dump(session_destroy());

// a second start while active
session_id('p8basic2');
session_start();
var_dump(session_start());
session_write_close();

// session_start with options
var_dump(session_start(['name' => 'P8NAME', 'cache_limiter' => '', 'read_and_close' => 1]));
var_dump(session_status() === PHP_SESSION_NONE, session_name());
session_name('PHPSESSID');

// the generated ids
var_dump(preg_match('/^[0-9a-v]{32}$/', session_create_id()));
var_dump(preg_match('/^pre-[0-9a-v]{32}$/', session_create_id('pre-')));

foreach (glob("$dir/sess_*") as $f) {
    unlink($f);
}
rmdir($dir);
