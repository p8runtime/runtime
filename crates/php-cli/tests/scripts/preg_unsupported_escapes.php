<?php
// PCRE2's own message for the escapes it does not support (\F \L \l \U \u
// \N{name}), and for an unknown one.
foreach (['/\F/', '/\L/', '/\l/', '/\U/', '/\u/', '/\N{x}/', '/[\N]/', '/\i/', '/a\Nb/'] as $p) {
    var_dump(@preg_match($p, 'aXb'), preg_last_error_msg());
    echo error_get_last()['message'] ?? '', "\n";
}
