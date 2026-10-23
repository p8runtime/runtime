<?php
$log = [];
set_error_handler(function ($no, $str, $file, $line) use (&$log) {
    $log[] = "$no: $str @" . $line;
    return true;
});
$a = [];
echo $a['missing'] ?? 'dflt', "\n";
$x = $a['nope'];
echo $undefinedVar;
trigger_error("custom", E_USER_WARNING);
print_r($log);
restore_error_handler();
$y = $a['shown'];
set_error_handler(function ($no, $str) { echo "handler: $str\n"; return false; }, E_WARNING);
$z = $a['both'];
restore_error_handler();
set_error_handler(function ($no, $str, $file, $line) { throw new ErrorException($str, 0, $no, $file, $line); });
try { $w = $a['thrown']; echo "not here\n"; } catch (ErrorException $e) { echo "caught: ", $e->getMessage(), "\n"; }
restore_error_handler();
spl_autoload_register(function ($class) {
    echo "autoload($class)\n";
    if ($class === 'Loaded') require __DIR__ . '/autoload_helper.inc';
});
echo Loaded::V, "\n";
echo Loaded::hi(), "\n";
$o = new Loaded; echo get_class($o), "\n";
var_dump(class_exists('Nope'), class_exists('Nope2', false), count(spl_autoload_functions()));
try { new Missing(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
class_alias('Loaded', 'AliasOfLoaded');
echo AliasOfLoaded::V, " ", constant('Loaded::V'), " ", constant('PHP_INT_SIZE'), "\n";
class P { static function who() { return get_called_class(); } } class Q extends P {}
echo Q::who(), "\n";
$g = 'global value';
register_shutdown_function(function ($arg) { global $g; echo "shutdown 1: $arg, $g\n"; }, 'arg');
register_shutdown_function(function () { echo "shutdown 2\n"; });
set_exception_handler(function ($e) { echo "uncaught handler: ", $e->getMessage(), "\n"; });
echo "end of script\n";
throw new RuntimeException("final");
