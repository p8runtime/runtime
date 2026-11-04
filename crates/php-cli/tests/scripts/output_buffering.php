<?php
var_dump(ob_get_level(), ob_get_contents());
ob_start(); echo "hello"; $c = ob_get_clean(); var_dump($c, ob_get_level());
ob_start(); echo "a"; ob_start(); echo "b"; var_dump(ob_get_level()); $inner = ob_get_contents(); ob_end_clean(); echo "+$inner"; $outer = ob_get_clean(); var_dump($outer);
ob_start(fn($buf, $phase) => strtoupper($buf) . "($phase)");
echo "shout "; ob_flush(); echo "more"; ob_end_flush(); echo "\n";
// (A named handler: a closure's name, {closure:file:line}, holds the path.)
function swap_x($b) { return str_replace('x', 'y', $b); }
ob_start('swap_x'); echo "xxx\n"; print_r(ob_list_handlers()); var_dump(ob_get_length());
ob_end_flush();
ob_start(); echo "clean me"; ob_clean(); echo "kept"; var_dump(ob_get_contents()); ob_end_clean();
var_dump(ob_end_clean(), ob_get_clean());
ob_start(); echo "flushed: "; var_dump(ob_get_flush());
ob_start(fn($b) => "[final:$b]"); echo "left open at the end";
