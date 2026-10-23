<?php
var_dump(sscanf("age: 25 name: Bob", "age: %d name: %s"));
var_dump(sscanf("12 apples", "%d %s %s"));
var_dump(sscanf("", "%d"));
var_dump(sscanf("abc", "%d"));
$n = sscanf("2024-01-15", "%4d-%2d-%2d", $y, $m, $d); var_dump($n, $y, $m, $d);
var_dump(sscanf("ff 0x1A 017 -3.5e2 x", "%x %x %o %f %c"));
var_dump(sscanf("hello world", "%[a-z] %[^d]"), sscanf("a,b,c", "%[^,],%[^,],%s"), sscanf("12345", "%2d%*d"), sscanf("abc 5", "%s %n%d"));
var_dump(sscanf("X", "%d"), sscanf("  42", "%d"), sscanf("1 2", "%2\$d %1\$d"));
$f = fopen('php://memory', 'w+'); fwrite($f, "10 20\nname Al\n"); rewind($f); var_dump(fscanf($f, "%d %d"), fscanf($f, "%s %s", $k, $v), $k, $v);
echo bin2hex(pack('nvc*', 0x1234, 0x5678, 65, 66)), " ", bin2hex(pack('NV', 1, 1)), " ", bin2hex(pack('a4A4Z4', 'ab', 'cd', 'ef')), " ", bin2hex(pack('H*h*', 'abc1', '12')), " ", bin2hex(pack('J', 1)), " ", strlen(pack('d', 1.5)), "\n";
print_r(unpack('nfirst/vsecond/c2chars', pack('nvc*', 0x1234, 0x5678, 65, -66)));
print_r(unpack('N', "\x00\x00\x01\x00")); print_r(unpack('a3x/A3y/H*hex', "abcde  \xff")); print_r(unpack('C*', "\x01\x02\x03")); print_r(unpack('q', pack('q', -5)));
var_dump(unpack('N', "ab"));
print_r(parse_ini_string("; comment\nname = Bob\nenabled = on\nflag = no\nnum = 42\nquoted = \"a = b\"\nlist[] = x\nlist[] = y\nmap[k] = v\n[sec]\ninside = 1\n"));
print_r(parse_ini_string("[one]\na=1\n[two]\nb=true\nc=null\nd=1.5\n", true, INI_SCANNER_TYPED)); var_dump(parse_ini_string("x = PHP_VERSION_ID")['x'] > 0);
try { assert(1 > 2); } catch (AssertionError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { assert(false, "custom msg"); } catch (AssertionError $e) { echo $e->getMessage(), "\n"; }
try { assert(false, new RuntimeException("rt")); } catch (RuntimeException $e) { echo get_class($e), $e->getMessage(), "\n"; }
var_dump(assert(true));
