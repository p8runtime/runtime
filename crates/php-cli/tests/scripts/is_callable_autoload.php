<?php
spl_autoload_register(function ($c) { echo "autoload $c\n"; if ($c === "AP") eval("class AP { static function d() {} }"); });
var_dump(is_callable(["AP", "d"]));
var_dump(is_callable("BP::d"));
var_dump(is_callable(["CP", "d"], true));
