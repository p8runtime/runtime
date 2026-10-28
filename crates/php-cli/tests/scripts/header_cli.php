<?php
ob_start();
var_dump(headers_sent($f, $l), $f, $l, http_response_code());
header('X-A: 1');
header('X-A: 2');
header('X-B: 1', false);
header('X-B: 2', false);
header('Location: /x');
var_dump(http_response_code());
header('X-C: 3', true, 404);
var_dump(http_response_code(500), http_response_code());
header('HTTP/1.1 403 Forbidden');
var_dump(http_response_code());
setcookie('a', 'b c', ['expires' => 0, 'path' => '/', 'samesite' => 'Lax', 'httponly' => true]);
setrawcookie('r', 'v');
var_dump(setcookie('x', '', 1));
header_remove('x-b');
print_r(headers_list());
header_remove();
print_r(headers_list());
try { header("X-Bad: a\nb"); } catch (Throwable $e) { echo get_class($e), $e->getMessage(), "\n"; }
header("X-Bad: a\nb");
header("NoColon");
print_r(headers_list());
try { setcookie('a=b', 'v'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { setcookie('', 'v'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { setcookie('n', 'v', ['bogus' => 1]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$buf = ob_get_clean(); echo $buf, "out\n";
var_dump(headers_sent($f, $l), basename($f), $l);
header('X-Late: 1');
var_dump(http_response_code(201));
print_r(headers_list());
header_remove('X-Y: z');
var_dump(setcookie('late', 'v'), setrawcookie('late', 'v'));
