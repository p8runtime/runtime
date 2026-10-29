<?php
// What a form request brings: the arrays, the uploads, the raw body.
$files = $_FILES;
array_walk_recursive($files, function (&$v, $k) {
    if ($k === 'tmp_name' && $v !== '') {
        $v = is_uploaded_file($v) ? 'uploaded:' . file_get_contents($v) : 'not-an-upload';
    }
});
$json = json_encode(['get' => $_GET, 'post' => $_POST, 'cookie' => $_COOKIE, 'request' => $_REQUEST, 'files' => $files, 'input' => file_get_contents('php://input'), 'method' => $_SERVER['REQUEST_METHOD']]);
// The temporary files' names are the only thing that varies.
echo preg_replace('~"[^"]*\\\\/php[A-Za-z0-9]{19}"~', '"TMP"', $json), "\n";
