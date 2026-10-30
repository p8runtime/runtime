<?php
// Arrays of literals are one constant (as PHP folds them): a literal list
// longer than a function's register space (wikimedia/common-passwords, 100,000
// entries) loads, and writing to a folded array never changes the constant.
function f() {
    foreach ([1, 2, 3] as &$v) { $v *= 10; }
    unset($v);
    $a = ['x' => [1, 2], 'y' => 'z'];
    $a['x'][] = 3; $a['n'] = 1;
    static $s = [1, 2];
    $s[] = count($s) + 1;
    $r = [5, 6];
    $ref = &$r[0]; $ref = 50;
    $keys = ['1' => 'a', '01' => 'b', true => 'd', null => 'e', 7 => 'f', 'g'];
    return [$a, $s, $r, $keys, [1, 2, 3]];
}
var_dump(f()); var_dump(f()[1], f()[4]);
$file = tempnam(sys_get_temp_dir(), 'lit');
$src = "<?php return [\n";
for ($i = 0; $i < 100000; $i++) { $src .= "'w$i' => 1,\n"; }
file_put_contents($file, $src . "'password' => 1, 'qwerty' => 2,\n];");
$list = include $file;
unlink($file);
var_dump(count($list), isset($list['password']), $list['qwerty']);
