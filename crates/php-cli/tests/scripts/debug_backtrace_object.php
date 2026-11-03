<?php
class TC { function run() { $m = 'test'; return $this->{$m}(); } function test() { $w = new W; return $w->get('x'); } }
class W { function get($k) { return Finder::find(); } }
class Finder { static function find() {
    foreach (debug_backtrace(DEBUG_BACKTRACE_PROVIDE_OBJECT | DEBUG_BACKTRACE_IGNORE_ARGS) as $f) {
        echo ($f['class'] ?? '-'), '::', $f['function'], ' ', isset($f['object']) ? get_class($f['object']) : 'no-object', "\n";
    }
} }
(new TC)->run();
eval('class E { function get($k) { return Finder::find(); } }');
(new E)->get(1);
$c = function () { return Finder::find(); };
