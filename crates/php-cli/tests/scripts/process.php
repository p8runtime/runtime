<?php
var_dump(is_string(PHP_BINARY) && PHP_BINARY !== '');
var_dump(shell_exec('echo hi'));
var_dump(shell_exec('true'));
$x = 'there';
var_dump(`echo hello $x`);
var_dump(exec('printf "a\nb  \nc\n"', $out, $rc), $out, $rc);
$o = ['z'];
exec('echo q', $o);
var_dump($o);
var_dump(system('echo sys; exit 3', $rc), $rc);
passthru('printf raw', $rc); echo "\n"; var_dump($rc);
var_dump(putenv('P8_T=1'), getenv('P8_T'), shell_exec('echo $P8_T'));
var_dump(putenv('P8_T'), getenv('P8_T'), array_key_exists('P8_T', getenv()));
try { putenv('=x'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { exec(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(getmypid() > 0, is_int(memory_get_usage()), is_int(memory_get_peak_usage(true)));
$old = get_include_path();
var_dump(set_include_path('/a:/b') === $old, get_include_path(), ini_get('include_path'));
