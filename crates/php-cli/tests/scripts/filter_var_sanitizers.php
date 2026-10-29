<?php
// filter_var's sanitizing filters, the callback filter and the array forms.
$s = "a<b>'\"&c\x01\x7f\xe9`";
foreach ([FILTER_UNSAFE_RAW, FILTER_SANITIZE_ENCODED, FILTER_SANITIZE_SPECIAL_CHARS, FILTER_SANITIZE_FULL_SPECIAL_CHARS,
    FILTER_SANITIZE_EMAIL, FILTER_SANITIZE_URL, FILTER_SANITIZE_NUMBER_INT, FILTER_SANITIZE_ADD_SLASHES] as $f) {
    echo $f, ' ', bin2hex((string) filter_var($s, $f)), "\n";
}
var_dump(filter_var($s, FILTER_UNSAFE_RAW, FILTER_FLAG_STRIP_LOW | FILTER_FLAG_STRIP_HIGH | FILTER_FLAG_STRIP_BACKTICK));
var_dump(filter_var($s, FILTER_UNSAFE_RAW, FILTER_FLAG_ENCODE_AMP | FILTER_FLAG_ENCODE_LOW | FILTER_FLAG_ENCODE_HIGH));
var_dump(filter_var('', FILTER_UNSAFE_RAW, FILTER_FLAG_EMPTY_STRING_NULL));
var_dump(filter_var('-1.5e3,000abc', FILTER_SANITIZE_NUMBER_FLOAT), filter_var('-1.5e3,000abc', FILTER_SANITIZE_NUMBER_FLOAT, FILTER_FLAG_ALLOW_FRACTION | FILTER_FLAG_ALLOW_THOUSAND | FILTER_FLAG_ALLOW_SCIENTIFIC));
var_dump(filter_var('"<x>"', FILTER_SANITIZE_FULL_SPECIAL_CHARS, FILTER_FLAG_NO_ENCODE_QUOTES));
var_dump(filter_var('MiXeD', FILTER_CALLBACK, ['options' => 'strtolower']));
try {
    filter_var('x', FILTER_CALLBACK, ['options' => 'no_such_function']);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
var_dump(filter_var_array(['id' => '7', 'mail' => 'bad', 'tags' => ['a', '2']], [
    'id' => FILTER_VALIDATE_INT,
    'mail' => ['filter' => FILTER_VALIDATE_EMAIL, 'flags' => FILTER_NULL_ON_FAILURE],
    'tags' => ['filter' => FILTER_VALIDATE_INT, 'flags' => FILTER_REQUIRE_ARRAY],
    'missing' => FILTER_DEFAULT,
]));
var_dump(filter_var_array(['a' => '1', 'b' => 'x'], FILTER_VALIDATE_INT));
var_dump(filter_var_array(['a' => '1'], ['missing' => FILTER_DEFAULT], false));
var_dump(filter_input(INPUT_GET, 'nothing'), filter_input(INPUT_GET, 'nothing', FILTER_DEFAULT, FILTER_NULL_ON_FAILURE));
var_dump(filter_input(INPUT_GET, 'x', FILTER_VALIDATE_INT, ['options' => ['default' => 3]]));
var_dump(filter_has_var(INPUT_GET, 'x'), filter_input_array(INPUT_POST));
var_dump(filter_id('validate_email'), filter_id('nope'), count(filter_list()));
var_dump(extension_loaded('filter'));
