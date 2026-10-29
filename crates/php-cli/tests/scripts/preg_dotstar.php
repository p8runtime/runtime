<?php
// A pattern starting with .* can only match at the start or after a
// newline (PCRE's dot-star optimisation): long lines don't exhaust the
// backtrack limit, and where matches start doesn't change.
// (PHPUnit's diff splits text with /(.*\R)/.)
foreach ([1000, 20000, 200000] as $n) {
    $r = preg_split('/(.*\R)/', "first\n" . str_repeat('x', $n), -1, PREG_SPLIT_DELIM_CAPTURE | PREG_SPLIT_NO_EMPTY);
    echo $n, ': ', count($r), ' ', preg_last_error_msg(), "\n";
}
var_dump(preg_match_all('/.*x/', "abx\ncdx\nef", $m, PREG_OFFSET_CAPTURE), $m);
var_dump(preg_match_all('/.*?x/', "abx\ncdx", $m), $m[0]);
var_dump(preg_match('/.*x/s', "ab\ncx", $m), $m);
var_dump(preg_match('/(?:.*a|.*b)c/', "zzz\nqbc", $m), $m);
var_dump(preg_match('/.*b/', "ab\nab", $m, PREG_OFFSET_CAPTURE, 4), $m);
var_dump(preg_match('/(.*)\1/', "abab\n", $m), $m);
