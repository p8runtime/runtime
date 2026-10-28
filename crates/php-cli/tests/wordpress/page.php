<?php
// A GET of getenv('WPURL') (path and query) through the right entry script.
$url = getenv('WPURL') ?: '/';
$path = parse_url($url, PHP_URL_PATH) ?: '/';
$query = parse_url($url, PHP_URL_QUERY) ?? '';
parse_str($query, $_GET);
$_REQUEST = $_GET;
$script = $path === '/' ? '/index.php' : $path;
$_SERVER['HTTP_HOST'] = 'localhost';
$_SERVER['REQUEST_URI'] = $url;
$_SERVER['QUERY_STRING'] = $query;
$_SERVER['SERVER_NAME'] = 'localhost';
$_SERVER['SERVER_PORT'] = '80';
$_SERVER['REQUEST_METHOD'] = 'GET';
$_SERVER['SCRIPT_NAME'] = $script;
$_SERVER['PHP_SELF'] = $script;
$_SERVER['SCRIPT_FILENAME'] = __DIR__ . '/wordpress' . $script;
chdir(dirname(__DIR__ . '/wordpress' . $script));
require __DIR__ . '/wordpress' . $script;
