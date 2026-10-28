<?php
ob_start();
header('WWW-Authenticate: Basic');
$a = http_response_code();
header('Location: /y', true, 307);
$b = http_response_code();
header('HTTP/1.0   500 x');
$c = http_response_code();
header('HTTP/1.1');
$d = http_response_code();
header("X-Trail: ok \r\n ");
header("X-Nul: a\0b");
header('');
header_remove('X-Y: z');
header_remove(null);
foreach ([['n', "a b"], ['n', 'v', ['path' => 'a;b']], ['n', 'v', ['domain' => "x\n"]], ['n', 'v', ['expires' => 253402300800]], ['n', 'v', [1 => 2]], ['n', 'v', ['EXPIRES' => 5, 'Secure' => 1, 'SameSite' => 'Lax']]] as $c2) {
    try { var_dump(setrawcookie(...$c2)); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
}
try { setcookie('n', 'v', [], '/'); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
var_dump(setcookie("n\0=x", 'v'), setcookie('n', "a b"));
$buf = ob_get_clean();
var_dump($a, $b, $c, $d);
echo $buf;
