<?php
// Every $_SERVER entry that does not depend on where or when it runs.
foreach ($_SERVER as $k => $v) {
    if (in_array($k, ['REMOTE_PORT', 'REMOTE_ADDR', 'REQUEST_TIME', 'REQUEST_TIME_FLOAT', 'DOCUMENT_ROOT', 'SCRIPT_FILENAME', 'HTTP_HOST', 'SERVER_NAME', 'SERVER_PORT', 'HTTP_USER_AGENT'])) {
        continue;
    }
    echo "$k=", is_array($v) ? json_encode($v) : $v, "\n";
}
echo basename($_SERVER['SCRIPT_FILENAME']), "\n";
echo php_sapi_name(), ' ', PHP_SAPI, ' ', basename(__FILE__), ' ', basename(getcwd()), "\n";
echo json_encode(function_exists('getallheaders')), "\n";
