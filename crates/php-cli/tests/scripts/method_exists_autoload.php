<?php
// method_exists/property_exists/get_parent_class autoload a class given by name, leading backslash and all (WooCommerce loads its merged packages so).
spl_autoload_register(function ($c) { echo "autoload $c\n"; if ($c === 'NS\Lazy') { eval('namespace NS; class Lazy { static function is_enabled() { return true; } }'); } });
var_dump(method_exists('\NS\Lazy', 'is_enabled'), method_exists('\NS\Lazy', 'nope'), method_exists('\NS\Missing', 'x'));
var_dump(call_user_func(['\NS\Lazy', 'is_enabled']), property_exists('\NS\Lazy', 'p'), class_exists('\NS\Lazy'));
var_dump(get_parent_class('\NS\Lazy'));
