<?php
// quoted_printable_encode()/decode() as quot_print.c does them (76-byte
// lines, soft breaks, a UTF-8 sequence kept on one line; PHPMailer uses
// them), get_defined_functions(), zend_version().
var_dump(zend_version());
$f = get_defined_functions();
var_dump(array_keys($f), in_array('strlen', $f['internal']), in_array('mb_strlen', $f['internal']), $f['user']);
function MyFunc() {}
if (true) { function later_one() {} }
var_dump(get_defined_functions()['user']);
var_dump(quoted_printable_encode("h\xe9llo = w\x00rld \r\n" . str_repeat('ab', 50) . "\xc3\xa9"));
var_dump(quoted_printable_encode(str_repeat('x', 73) . "\xe2\x82\xac" . str_repeat('y', 80) . "\xf0\x9f\x98\x80 \r\nend"));
var_dump(quoted_printable_decode("a=3Db=\r\nc= \nd=ZZ=4=e9=E9"), quoted_printable_decode("x=\ty"), quoted_printable_decode("stop\0here"));
foreach (["plain", "caf\xc3\xa9 = cr\xc3\xa8me", str_repeat("\xc3\xa9", 60)] as $s) {
    var_dump(quoted_printable_decode(quoted_printable_encode($s)) === $s);
}
