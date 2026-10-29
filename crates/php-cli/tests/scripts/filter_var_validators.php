<?php
// filter_var's validating filters, their flags and options.
$cases = [
    [FILTER_VALIDATE_INT, ['42', ' -7 ', '+0', '007', '9223372036854775808', '1e3', '', '0x1A', '0o17', '017']],
    [FILTER_VALIDATE_BOOL, ['yes', 'Off', ' TRUE ', '', 'maybe', '1', '0']],
    [FILTER_VALIDATE_FLOAT, ['1.5', '-1e3', '1,000.5', '.5', '1.', 'abc', '1e999', ' 2.25 ']],
    [FILTER_VALIDATE_EMAIL, ['ada@example.com', 'a.b+c@sub.example.co.uk', 'wordpress@localhost', 'no-at-sign', 'a@b..c', '"quoted"@example.com', 'x@[127.0.0.1]']],
    [FILTER_VALIDATE_URL, ['http://example.com/path?q=1', 'https://user:pw@host.example:8080/', 'mailto:ada@example.com', 'example.com', 'http://-bad.example/', 'http://[::1]/', 'ftp://files.example/x', 'http://ex ample.com']],
    [FILTER_VALIDATE_IP, ['127.0.0.1', '256.1.1.1', '01.2.3.4', '::1', '2001:db8::ff00:42:8329', '::ffff:192.0.2.1', '1::2::3', 'fe80::1', '10.0.0.1']],
    [FILTER_VALIDATE_MAC, ['01:23:45:67:89:ab', '01-23-45-67-89-AB', '0123.4567.89ab', '01:23:45:67:89', 'zz:23:45:67:89:ab']],
    [FILTER_VALIDATE_DOMAIN, ['example.com', 'a..b', str_repeat('a', 64) . '.com', '-x.example']],
];
foreach ($cases as [$filter, $values]) {
    foreach ($values as $v) {
        echo filter_id(array_search($filter, array_combine(filter_list(), array_map('filter_id', filter_list())))) ?: $filter, ' ', var_export($v, true), ' => ', var_export(filter_var($v, $filter), true), "\n";
    }
}
var_dump(filter_var('0x1A', FILTER_VALIDATE_INT, FILTER_FLAG_ALLOW_HEX));
var_dump(filter_var('017', FILTER_VALIDATE_INT, FILTER_FLAG_ALLOW_OCTAL));
var_dump(filter_var('5', FILTER_VALIDATE_INT, ['options' => ['min_range' => 1, 'max_range' => 4]]));
var_dump(filter_var('x', FILTER_VALIDATE_INT, ['options' => ['default' => 9]]));
var_dump(filter_var('maybe', FILTER_VALIDATE_BOOL, FILTER_NULL_ON_FAILURE));
var_dump(filter_var('1.000,5', FILTER_VALIDATE_FLOAT, ['options' => ['decimal' => ','], 'flags' => FILTER_FLAG_ALLOW_THOUSAND]));
var_dump(filter_var('10.0.0.1', FILTER_VALIDATE_IP, FILTER_FLAG_NO_PRIV_RANGE));
var_dump(filter_var('127.0.0.1', FILTER_VALIDATE_IP, FILTER_FLAG_NO_RES_RANGE));
var_dump(filter_var('::1', FILTER_VALIDATE_IP, FILTER_FLAG_IPV4));
var_dump(filter_var('http://example.com', FILTER_VALIDATE_URL, FILTER_FLAG_PATH_REQUIRED));
var_dump(filter_var('ad@exämple.com', FILTER_VALIDATE_EMAIL), filter_var('ädä@example.com', FILTER_VALIDATE_EMAIL, FILTER_FLAG_EMAIL_UNICODE));
var_dump(filter_var('-x.example', FILTER_VALIDATE_DOMAIN, FILTER_FLAG_HOSTNAME));
var_dump(filter_var('abc', FILTER_VALIDATE_REGEXP, ['options' => ['regexp' => '/^a/']]));
var_dump(filter_var(['1', 'x', ['2']], FILTER_VALIDATE_INT, FILTER_REQUIRE_ARRAY));
var_dump(filter_var('3', FILTER_VALIDATE_INT, FILTER_FORCE_ARRAY));
var_dump(filter_var(['1'], FILTER_VALIDATE_INT));
var_dump(filter_var(new stdClass, FILTER_VALIDATE_INT));
var_dump(filter_var(true, FILTER_VALIDATE_INT), filter_var(1.0, FILTER_VALIDATE_FLOAT));
var_dump(filter_var('x', 9999));
try {
    filter_var('x', FILTER_VALIDATE_REGEXP);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
