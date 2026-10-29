<?php
// The response head a script builds.
header('X-First: 1');
header('X-Many: a');
header('X-Many: b', false);
header('X-First: 2');
header('X-Gone: soon');
header_remove('X-Gone');
header('Content-Type: text/plain');
setcookie('plain', 'a b&c');
setcookie('opts', 'v', ['expires' => 2000000000, 'path' => '/p', 'secure' => true, 'httponly' => true, 'samesite' => 'Lax']);
setrawcookie('raw', 'a+b');
setcookie('del');
http_response_code(202);
var_dump(headers_sent());
echo json_encode(array_map(fn($h) => preg_replace('/Max-Age=\d+/', 'Max-Age=N', $h), headers_list())), "\n";
var_dump(headers_sent(), http_response_code());
