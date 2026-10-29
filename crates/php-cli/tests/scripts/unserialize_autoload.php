<?php
// unserialize() autoloads the classes it names (PHPUnit unserializes a
// separate process's result, whose classes the parent may not have loaded
// yet); one that still is not there is incomplete; a class not in
// allowed_classes is not autoloaded. __wakeup runs once.
spl_autoload_register(function ($c) {
    echo "autoload $c\n";
    if ($c === 'Later') {
        eval('class Later { public $v; function __wakeup() { echo "wakeup\n"; } }');
    }
});
$o = unserialize('a:2:{i:0;O:5:"Later":1:{s:1:"v";i:7;}i:1;O:5:"Never":0:{}}');
var_dump(get_class($o[0]), $o[0]->v, get_class($o[1]));
var_dump(get_class(unserialize('O:7:"Blocked":0:{}', ['allowed_classes' => false])));
var_dump(get_class(unserialize('O:5:"Later":0:{}')));
