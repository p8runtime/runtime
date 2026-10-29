<?php
// A request for getenv('WPURL') (path and query) through the right entry
// script: a GET, or a POST of the form in WPPOST (url-encoded) or of the
// raw body in WPRAW.
$url = getenv('WPURL') ?: '/';
$path = parse_url($url, PHP_URL_PATH) ?: '/';
$query = parse_url($url, PHP_URL_QUERY) ?? '';
parse_str($query, $_GET);
$form = getenv('WPPOST');
$raw = getenv('WPRAW');
if ($form !== false) {
    parse_str($form, $_POST);
    $_SERVER['CONTENT_TYPE'] = 'application/x-www-form-urlencoded';
}
if ($raw !== false) {
    $_SERVER['CONTENT_TYPE'] = 'text/xml';
    // A web server's body is php://input, which a command line does not
    // have: the XML-RPC server is handed it instead.
    $wp_filter['wp_xmlrpc_server_class'][10][] = [
        'function' => function () {
            class P8_Request_XMLRPC_Server extends wp_xmlrpc_server {
                public function serve_request() {
                    $this->IXR_Server($this->methods, getenv('WPRAW'));
                }
            }
            return 'P8_Request_XMLRPC_Server';
        },
        'accepted_args' => 0,
    ];
}
$_REQUEST = array_merge($_GET, $_POST);
$script = str_ends_with($path, '/') ? $path . 'index.php' : $path;
$_SERVER['HTTP_HOST'] = 'localhost';
$_SERVER['REQUEST_URI'] = $url;
$_SERVER['QUERY_STRING'] = $query;
$_SERVER['SERVER_NAME'] = 'localhost';
$_SERVER['SERVER_PORT'] = '80';
$_SERVER['REQUEST_METHOD'] = $form !== false || $raw !== false ? 'POST' : 'GET';
$_SERVER['SCRIPT_NAME'] = $script;
$_SERVER['PHP_SELF'] = $script;
$_SERVER['SCRIPT_FILENAME'] = __DIR__ . '/wordpress' . $script;
$_SERVER['REMOTE_ADDR'] = getenv('WPIP') ?: '127.0.0.1';
// rand() picks how antispambot() hides each e-mail address: seeded, it
// picks the same under both (p8's Mersenne Twister is PHP's).
mt_srand(1);
// No requests out to wordpress.org (update checks, the news widget).
define('WP_HTTP_BLOCK_EXTERNAL', true);
define('DISABLE_WP_CRON', true);
// Mail goes out from a real domain: PHPMailer refuses "wordpress@localhost".
$wp_filter['wp_mail_from'][10][] = [
    'function' => fn () => 'wordpress@example.com',
    'accepted_args' => 0,
];
// WPUSER=<id> requests as that user, signed in. The cookie check is a
// pluggable function, so defining it first replaces WordPress's own: both
// auth_redirect() in wp-admin and the current-user lookup go through it.
if (getenv('WPUSER')) {
    function wp_validate_auth_cookie($cookie = '', $scheme = '') {
        return (int) getenv('WPUSER');
    }
}
// WPNONCE=<action> sends the form's nonce for that action, as the page
// the form came from would have (made on init, once the user is known).
if (getenv('WPNONCE')) {
    $wp_filter['init'][999][] = [
        'function' => function () {
            $_POST['_wpnonce'] = $_REQUEST['_wpnonce'] = wp_create_nonce(getenv('WPNONCE'));
        },
        'accepted_args' => 0,
    ];
}
chdir(dirname(__DIR__ . '/wordpress' . $script));
require __DIR__ . '/wordpress' . $script;
