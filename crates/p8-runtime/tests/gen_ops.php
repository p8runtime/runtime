<?php
// Writes crates/p8-runtime/tests/fixtures/ops.txt: PHP 8.4's results for each binary
// operator on every pair of a value matrix, with the warnings, deprecations
// and exceptions raised. Run under real PHP:
//     docker run --rm -i php:8.4-cli php < crates/p8-runtime/tests/gen_ops.php > crates/p8-runtime/tests/fixtures/ops.txt
// The value list must match VALUES in crates/p8-runtime/tests/ops.rs.
$values = [null, true, false, 0, 1, -1, 7, -7, 3, PHP_INT_MAX, PHP_INT_MIN, 1.5, -2.5, 0.0, 7.5, INF,
    "", "0", "5", "1.5", "abc", "5 apples", " 3", "1e2", "ab", "AB", [], [1], [2 => 'x'], 64, 2];
$ops = ['+', '-', '*', '/', '%', '**', '.', '&', '|', '^', '<<', '>>'];
function show($v) {
    return match (true) {
        is_null($v) => 'N',
        is_bool($v) => 'b:' . ($v ? '1' : '0'),
        is_int($v) => "i:$v",
        is_float($v) => 'f:' . bin2hex(strrev(pack('d', $v))),
        is_string($v) => 's:' . bin2hex($v),
        is_array($v) => 'a:' . bin2hex(json_encode($v)),
    };
}
$diags = [];
set_error_handler(function ($no, $msg) use (&$diags) { $diags[] = ($no == E_DEPRECATED ? 'D:' : 'W:') . $msg; return true; });
foreach ($ops as $op) {
    $f = eval("return fn(\$a, \$b) => \$a $op \$b;");
    foreach ($values as $i => $a) {
        foreach ($values as $j => $b) {
            $diags = [];
            try {
                $r = show($f($a, $b));
            } catch (Throwable $e) {
                $r = 'E:' . get_class($e) . ':' . bin2hex($e->getMessage());
            }
            echo "$op $i $j $r", $diags ? ' ' . implode('|', array_map('bin2hex', $diags)) : '', "\n";
        }
    }
}
