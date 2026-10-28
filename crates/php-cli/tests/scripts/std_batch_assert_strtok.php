<?php
var_dump(array_change_key_case(['One' => 1, 'TWO' => 2, 3 => 3, 'one' => 4]));
var_dump(array_change_key_case(['a' => 1], CASE_UPPER));

$tok = strtok("  This is\tan example\nstring ", " \n\t");
while ($tok !== false) {
    echo "[$tok]\n";
    $tok = strtok(" \n\t");
}
var_dump(strtok("x"));

var_dump(E_STRICT);

function cb($file, $line, $null, $desc = null) { echo "callback: ", basename($file), " $line ", var_export($desc, true), "\n"; }
var_dump(@assert_options(ASSERT_CALLBACK, 'cb'));
var_dump(@assert_options(ASSERT_EXCEPTION, 0));
var_dump(assert(1 === 2));
var_dump(assert(false, 'custom'));
@assert_options(ASSERT_ACTIVE, 0);
var_dump(assert(false));
