<?php
// unserialize() looks each class up where it meets it: the autoloaders,
// then unserialize_callback_func, then __PHP_Incomplete_Class.
spl_autoload_register(function ($c) {
    echo "autoload $c\n";
    if ($c === 'Boom') throw new RuntimeException("no $c");
    if ($c === 'Suit') eval('enum Suit { case Hearts; const X = 1; }');
});
function cb($c) {
    echo "callback $c\n";
    if ($c === 'FromCb') eval('class FromCb { public $v; }');
}
var_dump(unserialize('a:2:{i:0;O:4:"Gone":0:{}i:1;O:4:"Gone":0:{}}'));
ini_set('unserialize_callback_func', 'cb');
var_dump(unserialize('O:6:"FromCb":1:{s:1:"v";i:3;}'));
var_dump(get_class(unserialize('O:5:"Still":0:{}')));
ini_set('unserialize_callback_func', 'nosuchfunc');
try { var_dump(unserialize('O:6:"Absent":0:{}')); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
ini_set('unserialize_callback_func', '');
try { var_dump(unserialize('a:1:{i:0;O:4:"Boom":0:{}}')); } catch (RuntimeException $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump(unserialize('E:11:"Suit:Hearts";'));
var_dump(unserialize('E:9:"Nope:Case";'));
var_dump(unserialize('E:6:"Suit:X";'));
var_dump(unserialize('E:6:"Suit:Y";'));
var_dump(unserialize('E:17:"stdClass:Whatever";'));
