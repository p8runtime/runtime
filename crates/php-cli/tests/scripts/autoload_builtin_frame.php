<?php
// An autoloader a builtin runs (zend_lookup_class) sees that builtin's
// frame below it, called from the line of the call; one an opcode runs
// (new, ::, instanceof) sees no internal frame.
function show() {
    foreach (debug_backtrace() as $f) {
        echo "  ", $f['function'], isset($f['line']) ? " @{$f['line']}" : " [internal]", isset($f['class']) ? " {$f['class']}" : "", "\n";
    }
}
spl_autoload_register(function ($c) {
    echo "autoload $c\n";
    show();
    if (str_starts_with($c, 'Have')) eval("#[Attribute] class $c { public \$v; static function m() {} function __construct() {} }");
});
var_dump(is_a('Missing1', 'Base', true));
var_dump(class_exists('Have1'));
var_dump(is_subclass_of('Missing2', 'Base'));
var_dump(method_exists('Missing3', 'm'));
var_dump(is_callable(['Nope', 'm']));
$o = unserialize('a:1:{i:0;O:5:"Have2":1:{s:1:"v";i:1;}}');
var_dump(get_class($o[0]));
try { new ReflectionClass('Missing4'); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
new Have3;
Have4::m();
var_dump(new stdClass instanceof Missing5);
function make() { return [new ReflectionClass("Have6")]; }
var_dump(make()[0]->getName());
#[Have5] class Attr {}
var_dump(get_class((new ReflectionClass('Attr'))->getAttributes()[0]->newInstance()));
array_map(function ($x) { show(); }, [1]);
