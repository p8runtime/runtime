<?php
// Literal strings and arrays are one value per isolate (made once, shared
// by every request): writes go to copies. Run twice in one isolate
// (--repeat 2) the output must be the same both times. Also array_merge's
// shortcuts (an empty side: the other array itself) and implode's.
class C {
    const LIST = ['a' => [1, 2], 'b' => 'text'];
    public $p = ['x' => ['y' => 1]];
    public static $s = ['n' => 0];
    public $str = 'property string';
}
function f() {
    $a = ['k' => ['deep' => 'v'], 'list' => [1, 2, 3]];
    $a['k']['deep'] .= '!';
    $a['list'][] = 4;
    $s = 'literal string here';
    $s[0] = 'L';
    $s .= ' more';
    return [$a, $s];
}
var_dump(f(), f());
$c = new C;
$c->p['x']['y']++;
$c->p['x']['z'] = 'new';
$c->str[0] = 'P';
C::$s['n']++;
$l = C::LIST;
$l['a'][] = 3;
$l['b'] .= '?';
var_dump($c, C::$s, C::LIST, $l, (new C)->p, (new C)->str);
$g = fn() => ['closure' => 'value'];
$x = $g();
$x['closure'] = 'changed';
var_dump($g(), $x);

$list = [1, 2, 3];
$m = array_merge([], $list);
$m[] = 4;
var_dump($m, $list, array_merge($list, []), array_merge(['a' => 1], []), array_merge([], ['a' => 1, 5 => 2]));
$holes = [0 => 'a', 2 => 'c'];
unset($list[2]);
var_dump(array_merge([], $holes), array_merge($list, []), array_merge([], $list));
$m2 = array_merge([], $list);
$m2[] = 'next';
var_dump($m2);
$cur = ['p', 'q', 'r'];
next($cur);
$m3 = array_merge($cur, []);
var_dump(current($m3), current($cur));
var_dump(array_merge(), array_merge(['x' => 1], ['y' => 2], [3]), array_merge(['k' => 'a'], ['k' => 'b', 0 => 'c']));

var_dump(implode(',', []), implode(',', ['one']), implode(',', [7]), implode(',', [1.5, true, null, 'x']), implode(['a', 'b']), implode(' ', ['just', 'two']));
$one = ['solo'];
$r = implode('-', $one);
$r .= '!';
var_dump($r, $one);
class T { function __toString(): string { return 'T!'; } }
var_dump(implode('|', [new T]), implode('|', [new T, 'x', 2]));
