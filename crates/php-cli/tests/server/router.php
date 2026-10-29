<?php
// A router: static files and the odd path are the server's, the rest is ours.
$path = parse_url($_SERVER['REQUEST_URI'], PHP_URL_PATH);
if (preg_match('/\.(txt|html)$/', $path)) {
    return false;
}
if ($path === '/script') {
    return false;
}
echo "routed ", $_SERVER['REQUEST_URI'], " ", $_SERVER['SCRIPT_NAME'], " ", basename($_SERVER['SCRIPT_FILENAME']), "\n";
