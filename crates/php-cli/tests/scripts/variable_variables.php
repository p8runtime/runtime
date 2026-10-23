<?php
$name = 'greeting';
$$name = 'hello';
var_dump($greeting, $$name, ${'gree' . 'ting'});
$n = 'count';
$$n = 1;
$$n++;
$$n += 10;
var_dump($count, isset($$n), isset($$name), isset(${'nope'}));
$arr = 'list';
$$arr[] = 'a';
${$arr}['k'] = 'b';
var_dump($list);
unset($$n);
var_dump(isset($count));
function f($v) {
    $x = 'dyn';
    $$x = $v;
    ${'other'} = 2;
    $$x .= '!';
    var_dump($$x, isset($$x), get_defined_vars());
    unset($$x);
    var_dump(isset($$x), compact('other'));
    $k = 'v';
    return $$k;
}
var_dump(f('val'));
function g() { extract(['made' => 'by extract']); $m = 'made'; return $$m; }
var_dump(g());
function inc(&$r) { $r++; }
$z = 'num'; $num = 5;
inc($$z);
var_dump($num);
${1} = 'one';
${true} = 'bool';
var_dump(${'1'});
echo $$undefinedname ?? 'default', "\n";
$q = 'missing';
var_dump($$q);
foreach ([1, 2] as $i) { ${"v$i"} = $i * 10; }
var_dump($v1, $v2);
