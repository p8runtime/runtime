<?php
$a = fopen('php://memory', 'w+'); fwrite($a, "0123456789"); rewind($a);
$b = fopen('php://memory', 'w+');
var_dump(stream_copy_to_stream($a, $b, 4, 2), stream_copy_to_stream($a, $b), stream_copy_to_stream($a, $b));
rewind($b); var_dump(stream_get_contents($b));
var_dump(ip2long('192.168.1.1'), ip2long('255.255.255.255'), ip2long('1.2.3'), ip2long('::1'), ip2long(''));
var_dump(long2ip(3232235777), long2ip(-1), long2ip(0));
$f = tempnam(sys_get_temp_dir(), 'st');
file_put_contents($f, "<?php\n// a comment\n/** doc */\nfunction  f( \$x ) {   # trailing\n    return \$x  +  1; /* c */\n}\n\$s = <<<EOT\n  keep   this\n  EOT;\necho  f(1);\n");
var_dump(php_strip_whitespace($f));
unlink($f);
