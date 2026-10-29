<?php
// Invalid UTF-8: an empty result, unless ENT_SUBSTITUTE (U+FFFD for each
// bad sequence, cut as PHP cuts them) or ENT_IGNORE (dropped).
$inputs = ["a\xe9b", "\xc3", "\xc3\x28", "\xe2\x82", "\xe2\x28\xa1", "\xed\xa0\x80", "\xe0\x80\xaf", "\xf0\x90\x80", "\xf4\x90\x80\x80", "\xf8\x88\x80\x80\x80", "\x80\xbf", "\xc3\xa9<\xe2\x82\xac>"];
foreach ($inputs as $s) {
    $row = [];
    foreach ([ENT_QUOTES, ENT_QUOTES | ENT_SUBSTITUTE, ENT_QUOTES | ENT_IGNORE] as $f) {
        $row[] = bin2hex(htmlspecialchars($s, $f));
        $row[] = bin2hex(htmlentities($s, $f));
    }
    echo bin2hex($s), ': ', implode(' ', $row), "\n";
}
// The defaults substitute; a single-byte charset is not checked.
echo bin2hex(htmlspecialchars("a\xe9b")), ' ', bin2hex(htmlentities("a\xe9b")), ' ', bin2hex(htmlspecialchars("a\xe9b", ENT_QUOTES, 'ISO-8859-1')), "\n";
