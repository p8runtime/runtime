<?php
// The files handler: strict mode, regenerating the id, a save path with a
// depth and a mode, garbage collection by age, and the failures.
ob_start();
ini_set('session.gc_probability', '0');
ini_set('session.cache_limiter', '');
$dir = sys_get_temp_dir() . '/p8_session_files_' . getmypid();
mkdir($dir);
mkdir("$dir/a");
mkdir("$dir/z");

function files(string $dir): array
{
    $found = array_map(fn($f) => substr($f, strlen($dir) + 1), glob("$dir/{,*/}sess_*", GLOB_BRACE));
    sort($found);
    return $found;
}

echo "== strict mode\n";
session_save_path($dir);
ini_set('session.use_strict_mode', '1');
session_id('nonexistent');
session_start();
var_dump(session_id() === 'nonexistent', strlen(session_id()));
$_SESSION['k'] = 1;
$real = session_id();
session_write_close();
session_id($real);
session_start();
var_dump(session_id() === $real, $_SESSION);
session_write_close();
ini_set('session.use_strict_mode', '0');
var_dump(count(files($dir)));

echo "== regenerate\n";
session_id('regen1');
session_start();
$_SESSION['r'] = 1;
$keep = session_regenerate_id(false);
$one = session_id();
$_SESSION['r'] = 2;
$drop = session_regenerate_id(true);
$two = session_id();
var_dump($keep, $drop, $one !== 'regen1', $two !== $one);
session_write_close();
var_dump(file_exists("$dir/sess_regen1"), file_exists("$dir/sess_$one"), file_exists("$dir/sess_$two"));
var_dump(file_get_contents("$dir/sess_regen1"), file_get_contents("$dir/sess_$two"));

echo "== depth and mode\n";
session_save_path("1;0640;$dir");
var_dump(session_save_path() === "1;0640;$dir");
session_id('zdeep');
session_start();
$_SESSION['d'] = 1;
session_write_close();
var_dump(file_exists("$dir/z/sess_zdeep"));
clearstatcache();
var_dump(decoct(fileperms("$dir/z/sess_zdeep") & 0777));
session_save_path("x;$dir");
session_id('bad1');
var_dump(@session_start());
session_save_path("1;99999999999999999999;$dir");
session_id('bad2');
var_dump(@session_start());

echo "== garbage collection\n";
session_save_path($dir);
ini_set('session.gc_maxlifetime', '100');
touch("$dir/sess_old1", time() - 1000);
touch("$dir/sess_old2", time() - 1000);
touch("$dir/sess_fresh", time());
touch("$dir/other_file", time() - 1000);
session_id('gc1');
session_start();
var_dump(session_gc());
session_abort();
var_dump(file_exists("$dir/sess_old1"), file_exists("$dir/sess_fresh"), file_exists("$dir/other_file"));
var_dump(session_gc());

echo "== a file that cannot be written\n";
session_save_path("$dir/missing/dir");
session_id('nowrite');
var_dump(@session_start());
echo preg_replace('~\(path: .*\)~', '(path: ...)', error_get_last()['message']), "\n";
var_dump(session_status() === PHP_SESSION_NONE, session_id());
session_save_path($dir);
session_id('has/slash');
var_dump(@session_start());
echo preg_replace('~\(path: .*\)~', '(path: ...)', error_get_last()['message']), "\n";

foreach (array_merge(glob("$dir/sess_*"), glob("$dir/z/sess_*"), ["$dir/other_file"]) as $f) {
    unlink($f);
}
rmdir("$dir/z");
rmdir("$dir/a");
rmdir($dir);
