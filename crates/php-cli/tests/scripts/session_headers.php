<?php
// Under the CLI no header goes out, but the checks do: output sent before the
// session starts, and the cookie or cache headers that cannot follow it.
ini_set('session.gc_probability', '0');
$dir = sys_get_temp_dir() . '/p8_session_head_' . getmypid();
mkdir($dir);
session_save_path($dir);

ob_start();
var_dump(headers_list());
session_id('p8head1');
var_dump(session_start());
var_dump(headers_list());
var_dump(session_status() === PHP_SESSION_ACTIVE);
session_write_close();

// a destroyed session leaves no id: the next one takes it from the request.
// The id from the cookie is kept and no cookie is sent.
session_start();
session_destroy();
$_COOKIE['PHPSESSID'] = 'p8head-cookie';
var_dump(session_start());
var_dump(session_id());
var_dump(headers_list());
session_destroy();
unset($_COOKIE['PHPSESSID']);

// an id in the query is ignored while use_only_cookies is on
$_GET['PHPSESSID'] = 'p8head-get';
session_start();
var_dump(session_id() === 'p8head-get');
session_destroy();
unset($_GET['PHPSESSID']);

// a hostile id is not used
$_COOKIE['PHPSESSID'] = "bad id<>";
session_start();
var_dump(session_id() === "bad id<>", strlen(session_id()));
session_destroy();
$_COOKIE['PHPSESSID'] = ['array'];
session_start();
var_dump(strlen(session_id()));
session_destroy();
unset($_COOKIE['PHPSESSID']);

// SID
session_id('p8head1');
session_start();
var_dump(@SID);
session_write_close();

// output has started: the session cannot start, and its settings are fixed
ob_end_flush();
echo "output\n";
var_dump(headers_sent());
var_dump(session_start());
var_dump(session_status() === PHP_SESSION_NONE);
var_dump(session_id('p8head2'));
var_dump(session_id());
var_dump(session_name('X'));
var_dump(session_save_path($dir));
var_dump(session_cache_limiter('public'));
var_dump(session_cache_expire(10));
var_dump(session_module_name('files'));
var_dump(session_set_cookie_params(1));
var_dump(ini_set('session.name', 'X'));
var_dump(session_regenerate_id());
var_dump(session_get_cookie_params()['lifetime']);

// with cookies off the id may change after output
ini_set('session.use_cookies', '0');
var_dump(session_id('p8head3'));
var_dump(session_start());
var_dump(session_status() === PHP_SESSION_ACTIVE);
session_write_close();
var_dump(session_cache_limiter());

foreach (glob("$dir/sess_*") as $f) {
    unlink($f);
}
rmdir($dir);
