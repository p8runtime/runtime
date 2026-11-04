<?php
// Strings built in one buffer: concatenation chains and interpolation of
// every kind of value (conversion order, __toString timing, warnings),
// `.=` in loops, and builtins that hand back their input when nothing
// changes (strtolower, str_replace, htmlspecialchars, substr,
// preg_replace) or build a new string (implode, strtr, json_decode,
// unserialize, (array) casts). (NUL bytes are printed as \0.)
ob_start(fn($b) => str_replace("\0", '\0', $b));
class T {
    function __construct(public $n) {}
    function __toString(): string { echo "[toString {$this->n}]"; return "T{$this->n}"; }
}
$i = 42; $f = 1.5; $b = true; $n = null; $s = 'str';
$t1 = new T(1); $t2 = new T(2);
echo $s . $i . $f . $b . $n . 'end', "\n";
echo "a{$i}b{$f}c{$b}d{$n}e$s", "\n";
echo "x" . $t1 . "y" . $t2 . "z", "\n";
echo "obj {$t1} and {$t2} done\n";
$arr = [1];
echo @("arr: " . $arr), "\n";
echo "arr: " . $arr . "\n";
echo "in $arr[0] and {$arr[0]}\n";
$short = 'ab' . 'cd' . $i;
$long = 'a long piece of text ' . $i . ' and some more ' . $f . ' to pass the inline size';
var_dump($short, $long, strlen($long));
$acc = '';
for ($k = 0; $k < 50; $k++) { $acc .= $k . ','; }
var_dump(strlen($acc), substr($acc, 0, 20), substr($acc, -10));
$acc2 = 'x';
for ($k = 0; $k < 5; $k++) { $acc2 .= $t1; }
var_dump($acc2);
$copy = $acc; $copy .= '!'; var_dump(strlen($acc), strlen($copy));

var_dump(strtolower('already lower'), strtolower('MiXeD Case ÄÖ'), strtoupper('UPPER'), strtoupper('lower 123 ß'));
var_dump(str_replace('zz', 'yy', 'nothing here'), str_replace('a', 'bbb', 'banana'), str_replace(['a', 'b'], ['b', 'c'], 'ab'), str_replace('a', '', 'aaa', $count), $count);
var_dump(str_ireplace('A', 'x', 'aAa'), str_replace([], [], 'same'), str_replace('', 'x', 'empty search'));
var_dump(htmlspecialchars('plain text, nothing to escape'), htmlspecialchars('<a href="x">\'q\' & co</a>'), htmlspecialchars("bad \xff utf8"), htmlspecialchars("bad \xff utf8", ENT_SUBSTITUTE), htmlspecialchars("bad \xff", ENT_IGNORE), htmlspecialchars("it's", ENT_NOQUOTES), htmlspecialchars('&amp; &lt;', ENT_QUOTES, 'UTF-8', false));
var_dump(substr('whole', 0), substr('whole', 0, 5), substr('whole', 1, 3), substr('whole', 10), substr('', 0));
var_dump(preg_replace('/zz/', 'y', 'no match here'), preg_replace('/a/', 'b', 'banana', 0), preg_replace('/a/', 'b', 'banana', 1), preg_replace_callback('/q/', fn($m) => 'Q', 'no q? yes q'));
var_dump(implode(',', ['a', 2, 3.5, true, null]), implode('', []), implode('-', ['solo']), implode('-', [7]), implode(', ', [$t1, 'x', $t2]));
var_dump(strtr('hello world', ['hello' => 'bye', 'o' => '0']), strtr('abc', ['zz' => 'y']), strtr('abc', 'ab', 'xy'));
var_dump(json_decode('"plain"'), json_decode('"esc\\n\\u00e9\\"q"'), json_decode('["a","bb",{"k":"a value long enough to be on the heap"}]', true), json_decode("\"bad \xff\""), json_last_error(), json_decode("\"ctl \x01\""), json_last_error(), json_decode('"\\ud83d\\ude00"'));
var_dump(unserialize(serialize(['abc', 'k' => 'a string long enough to be on the heap', '', "nul\0inside"])), unserialize('a:1:{i:0;s:5:"short";}'), unserialize('O:7:"Missing":1:{s:1:"p";s:1:"v";}'));
class P { public $pub = 1; protected $pro = 2; private $pri = 3; public $long_public_property_name_here = 4; }
$o = new P; $o->{'5'} = 'five'; $o->dyn = 'd';
$cast = (array) $o;
var_dump(count($cast), array_map('strlen', array_keys($cast)), $cast["\0*\0pro"], $cast["\0P\0pri"], $cast[5] ?? 'no int key');
var_dump(get_object_vars($o));
