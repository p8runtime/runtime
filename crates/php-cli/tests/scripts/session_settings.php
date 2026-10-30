<?php
// The session.* settings: what ini_set() and the session functions that set
// them refuse, with PHP's warnings.
ob_start();
ini_set('session.gc_probability', '0');
$dir = sys_get_temp_dir() . '/p8_session_set_' . getmypid();
mkdir($dir);
session_save_path($dir);

echo "== values\n";
foreach (['session.name' => ['', '123', 'a b', 'a=b', 'a.b', "a\0b", 'P8'],
          'session.save_handler' => ['nope', 'user', 'files'],
          'session.serialize_handler' => ['nope', 'php_binary', 'php'],
          'session.cookie_lifetime' => ['-1', '3600', '0'],
          'session.sid_length' => ['10', '300', '40x', '32'],
          'session.sid_bits_per_character' => ['3', '7', '5', '4'],
          'session.gc_probability' => ['-1', '1', '0'],
          'session.gc_divisor' => ['0', '-5', '100'],
          'session.auto_start' => ['1'],
          'session.upload_progress.freq' => ['5%']] as $name => $list) {
    foreach ($list as $value) {
        $old = ini_set($name, $value);
        echo $name, ' = ', addcslashes($value, "\0..\37"), ' -> ', var_export($old, true), ' now ', var_export(ini_get($name), true), "\n";
    }
}

echo "== while a session is active\n";
session_id('p8settings');
session_start();
var_dump(ini_set('session.name', 'ACTIVE'));
var_dump(ini_set('session.cookie_path', '/x'));
var_dump(session_name('ACTIVE'));
var_dump(session_module_name('files'));
var_dump(session_save_path('/nowhere'));
var_dump(session_cache_limiter('public'));
var_dump(session_cache_expire(5));
var_dump(session_set_cookie_params(10));
var_dump(session_id('other'));
var_dump(session_name());
session_write_close();

echo "== session_name, session_module_name, session_save_path\n";
var_dump(session_name('P8NAME'));
var_dump(session_name());
var_dump(session_name('PHPSESSID'));
try {
    var_dump(session_name("a\0b"));
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
try {
    var_dump(session_module_name('user'));
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
var_dump(session_module_name('nope'));
var_dump(session_module_name('files'));
try {
    var_dump(session_save_path("a\0b"));
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}

echo "== cache limiter, expire\n";
var_dump(session_cache_limiter(), session_cache_expire());
var_dump(session_cache_limiter('private'), session_cache_limiter());
var_dump(session_cache_expire(7), session_cache_expire());
session_cache_limiter('nocache');
session_cache_expire(180);

echo "== cookie parameters\n";
var_dump(session_get_cookie_params());
var_dump(session_set_cookie_params(3600, '/p', 'example.test', true, true));
var_dump(session_get_cookie_params());
var_dump(session_set_cookie_params(['lifetime' => 10, 'samesite' => 'Strict', 'bogus' => 1, 5 => 'x']));
var_dump(session_get_cookie_params());
foreach ([[['bogus' => 1]], [[], '/x'], [['lifetime' => 1], null, 'd'], [['lifetime' => 1], null, null, true], [['lifetime' => 1], null, null, null, false], [-5]] as $args) {
    try {
        var_dump(@session_set_cookie_params(...$args));
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
}
ini_set('session.use_cookies', '0');
var_dump(session_set_cookie_params(1));
ini_set('session.use_cookies', '1');

echo "== deprecated\n";
var_dump(ini_set('session.use_only_cookies', '0'));
var_dump(ini_set('session.use_only_cookies', '1'));
var_dump(ini_set('session.use_trans_sid', '1'));
var_dump(ini_set('session.use_trans_sid', '0'));
var_dump(ini_set('session.referer_check', 'x'));
var_dump(ini_set('session.referer_check', ''));
var_dump(ini_set('session.sid_length', '40'));
var_dump(ini_set('session.sid_length', '32'));
var_dump(ini_set('session.sid_bits_per_character', '5'));
var_dump(ini_set('session.sid_bits_per_character', '4'));
var_dump(ini_set('session.trans_sid_tags', 'a=href'));
var_dump(ini_set('session.trans_sid_hosts', 'example.test'));

foreach (glob("$dir/sess_*") as $f) {
    unlink($f);
}
rmdir($dir);
