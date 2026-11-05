<?php
// Writes crates/p8-runtime/tests/fixtures/semantics.txt: how PHP 8.6 compares and converts a
// matrix of values. Run under real PHP:
//     docker run --rm -i php:8.6-cli php < crates/p8-runtime/tests/gen_semantics.php > crates/p8-runtime/tests/fixtures/semantics.txt
// The value list must match VALUES in crates/p8-runtime/tests/semantics.rs.
// Only the values are recorded: the diagnostics some conversions raise
// (8.5+: NAN coerced, a float not representable as an int) are the
// operators' and casts' business, checked in ops.txt and by the php-src suite.
set_error_handler(fn () => true);
$values = [null, true, false, 0, 1, -1, 1.5, -0.0, INF, NAN, PHP_INT_MAX, 9.2233720368547758E18,
    "", "0", "1", "1.0", "abc", "1e3", " 1", "1 ", "01", "-1", "a", "A", "9223372036854775808", " ", "0.0",
    [], [1], [1, 2], ["a" => 1], [1 => 2, 0 => 1], [0 => 1, 1 => 2], ["a" => 1, "b" => 2], ["b" => 2, "a" => 1]];
foreach ($values as $i => $a) {
    $s = is_array($a) ? 'ARRAY' : bin2hex((string) $a);
    printf("conv %d %s %d %s %s\n", $i, var_export((bool) $a, true), is_array($a) ? (int) (bool) $a : (int) $a,
        is_array($a) ? 'x' : bin2hex(pack('E', (float) $a)), $s);
}
foreach ($values as $i => $a) {
    foreach ($values as $j => $b) {
        printf("cmp %d %d %d %d %d\n", $i, $j, $a <=> $b, $a == $b ? 1 : 0, $a === $b ? 1 : 0);
    }
}
