<?php
// ini settings, error levels, trigger_error, runtime information.
var_dump(ini_get("error_reporting"), ini_get("display_errors"), ini_get("precision"), ini_get("nope"), error_reporting(), ini_get("memory_limit"), ini_get("max_execution_time"));
var_dump(ini_set("precision", "10"), ini_get("precision"), ini_set("nope", "1"), ini_set("precision", 14));
var_dump(error_reporting(E_ALL & ~E_WARNING), error_reporting(), ini_get("error_reporting"));
echo $undefined;
error_reporting(E_ALL);
echo $undefined2;
var_dump(php_sapi_name(), PHP_VERSION === phpversion(), phpversion("standard") === PHP_VERSION, phpversion("nope"), set_time_limit(5), ini_get("max_execution_time"), gc_collect_cycles(), gc_enabled());
var_dump(is_int(time()), getenv("P8_SURELY_UNSET"), is_array(getenv()));
trigger_error("custom notice");
trigger_error("custom warning", E_USER_WARNING);
trigger_error("custom deprecated", E_USER_DEPRECATED);
ini_set("display_errors", "0");
trigger_error("hidden");
ini_set("display_errors", "1");
error_reporting(E_ALL & ~E_USER_NOTICE);
trigger_error("filtered");
error_reporting(E_ALL);
trigger_error("fatal", E_USER_ERROR);
echo "not reached\n";
