<?php
foreach (['ctype_alnum', 'ctype_alpha', 'ctype_cntrl', 'ctype_digit', 'ctype_lower', 'ctype_graph', 'ctype_print', 'ctype_punct', 'ctype_space', 'ctype_upper', 'ctype_xdigit'] as $f) {
    $out = [];
    foreach (['', 'abc', 'ABC', 'a1', '123', "\t\n", ' ', '!?', "\x7f", 'ff0A', "\xe4", -129, -1, 65, 256, 1000, 1.5, null, true, []] as $v) {
        $out[] = @$f($v) ? 'y' : 'n';
    }
    echo $f, ' ', implode('', $out), "\n";
}
var_dump(ctype_digit(5));
var_dump(extension_loaded('ctype'), extension_loaded('CTYPE'), extension_loaded('xmlwriter'), extension_loaded('nope'));
