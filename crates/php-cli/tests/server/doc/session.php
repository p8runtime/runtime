<?php
// A session under the built-in server: the cookie and the cache headers it
// adds, and an id the request brings back.
ob_start();
session_save_path(sys_get_temp_dir());
if (isset($_GET['limiter'])) {
    session_cache_limiter($_GET['limiter']);
}
if (isset($_GET['params'])) {
    session_set_cookie_params(['path' => '/app', 'domain' => 'example.test', 'secure' => true, 'httponly' => true, 'samesite' => 'Strict']);
}
if (isset($_GET['setcookie'])) {
    setcookie('other', 'x');
}
if (isset($_GET['id'])) {
    session_id($_GET['id']);
}
$started = session_start();
$_SESSION['seen'] = true;
function shown(): array
{
    return preg_replace('/PHPSESSID=(?!p8server)[^;]+/', 'PHPSESSID=NEWID', headers_list());
}
echo json_encode(['started' => $started, 'id' => session_id(), 'headers' => shown()]), "\n";
if (isset($_GET['regenerate'])) {
    session_regenerate_id(true);
    echo json_encode(['regenerated' => session_id() !== $_GET['id'], 'headers' => shown()]), "\n";
    // (the new id is random)
    header_remove('Set-Cookie');
}
session_destroy();
