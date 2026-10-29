<?php
// A call with ...$spread still passes earlier arguments by reference where
// the callee takes them that way: variables, elements and properties.
class W {
    function el(&$out, $e, $d = 0, $args = []) { $out .= "[$e:$d]"; }
    function disp($e, &$kids, $d, $args, &$out) {
        $this->el($out, $e, $d, ...array_values($args));
        if (isset($kids[$e])) {
            foreach ($kids[$e] as $c) {
                $this->disp($c, $kids, $d + 1, $args, $out);
            }
        }
    }
    function walk($els, ...$args) {
        $out = '';
        $kids = ['a' => ['b']];
        foreach ($els as $e) {
            $this->disp($e, $kids, 0, $args, $out);
        }
        return $out;
    }
}
echo (new W)->walk(['a'], ['x' => 1]), "\n";
function g(&$o, ...$r) { $o .= count($r); }
$s = 'n';
g($s, ...[1, 2]);
$f = 'g';
$f($s, ...[1]);
echo $s, "\n";
$a = [1];
$items = [2, 3];
array_push($a, ...$items);
$b = ['x' => [5]];
array_push($b['x'], ...$items);
class O { public $l = []; }
$o = new O;
array_unshift($o->l, ...$items);
var_dump($a, $b, $o->l);
function byval($x, ...$r) { $x .= '!'; return $x; }
$v = 'v';
echo byval($v, ...[1]), $v, "\n";
$arr = ['k' => 'a'];
echo byval($arr['k'], ...[]), "\n";
var_dump($arr);
function byref(&$x, ...$r) { $x = 'set'; }
byref($new, ...[]);
$m = ['p' => 1];
byref($m['q'], ...[]);
var_dump($new, $m);
