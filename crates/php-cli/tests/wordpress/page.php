<?php
// A GET of getenv('WPURL') (path and query) through the right entry script.
$url = getenv('WPURL') ?: '/';
$path = parse_url($url, PHP_URL_PATH) ?: '/';
$query = parse_url($url, PHP_URL_QUERY) ?? '';
parse_str($query, $_GET);
$_REQUEST = $_GET;
$script = str_ends_with($path, '/') ? $path . 'index.php' : $path;
$_SERVER['HTTP_HOST'] = 'localhost';
$_SERVER['REQUEST_URI'] = $url;
$_SERVER['QUERY_STRING'] = $query;
$_SERVER['SERVER_NAME'] = 'localhost';
$_SERVER['SERVER_PORT'] = '80';
$_SERVER['REQUEST_METHOD'] = 'GET';
$_SERVER['SCRIPT_NAME'] = $script;
$_SERVER['PHP_SELF'] = $script;
$_SERVER['SCRIPT_FILENAME'] = __DIR__ . '/wordpress' . $script;
$_SERVER['REMOTE_ADDR'] = '127.0.0.1';
// No requests out to wordpress.org (update checks, the news widget).
define('WP_HTTP_BLOCK_EXTERNAL', true);
define('DISABLE_WP_CRON', true);
// WPUSER=<id> requests as that user, signed in. The cookie check is a
// pluggable function, so defining it first replaces WordPress's own: both
// auth_redirect() in wp-admin and the current-user lookup go through it.
if (getenv('WPUSER')) {
    function wp_validate_auth_cookie($cookie = '', $scheme = '') {
        return (int) getenv('WPUSER');
    }
}
chdir(dirname(__DIR__ . '/wordpress' . $script));
require __DIR__ . '/wordpress' . $script;
