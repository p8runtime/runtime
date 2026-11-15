<?php
// serialize() writes a PHP reference met a second time as `R:N;` (N: the
// number of the value it was first written as), an object met again as
// `r:N;`; unserialize() reads `R:N;` back as the same reference and `r:N;`
// as the same object (#552).
function show($v) {
    echo str_replace("\0", '~', serialize($v)), "\n";
}

// Two elements of one array, one reference.
$a = [1];
$a[1] = &$a[0];
show($a);
$u = unserialize(serialize($a));
$u[1] = 9;
echo $u[0], "\n";

// The reference deeper in, and a copy of its value beside it.
$x = 5;
$b = ['p' => &$x, 'q' => [&$x, 'r' => $x], 's' => [[&$x]]];
show($b);
$u = unserialize(serialize($b));
$u['s'][0][0] = 'changed';
echo $u['p'], ' ', $u['q'][0], ' ', $u['q']['r'], "\n";

// Two copies of an array holding a reference: the copies are written in
// full, the reference inside the second is R:.
$y = 1;
$inner = [&$y, 2];
show([$inner, $inner]);

// Two references, each written twice; values between them are counted.
$m = 'm';
$n = 'n';
show([&$m, 'x', &$n, [true, null, 1.5], &$m, &$n]);

// A reference counted once: what follows R: numbers on from it.
$k = 1;
$o = new stdClass;
show([&$k, &$k, $o, $o]);

// A reference to an object: a repeat of the reference is R:, the object
// itself again is r: (both name the object's number).
$o = new stdClass;
$o->v = 1;
$c = [&$o, &$o, $o];
show($c);
$d = [$o];
$d[1] = &$o;
show($d);
$u = unserialize(serialize($c));
var_dump($u[0] === $u[2]);
$u[1] = 'replaced';
var_dump($u[0]);

// Objects: r: takes a number of its own.
$p = new stdClass;
$q = new stdClass;
$q->name = 'q';
$s = serialize([$p, $p, $q, $q]);
echo $s, "\n";
$u = unserialize($s);
var_dump($u[0] === $u[1], $u[2] === $u[3], $u[0] !== $u[2], $u[3]->name);

// A property referring to another.
$o = new stdClass;
$o->a = 1;
$o->b = &$o->a;
show($o);
$u = unserialize(serialize($o));
$u->b = 2;
echo $u->a, "\n";

// A reference to a variable holding the array that contains it.
$r = [];
$r[0] = &$r;
show($r);
$u = unserialize(serialize($r));
var_dump(is_array($u[0][0][0][0]));

// A reference only one place holds is written as its value, also where
// that place is written twice.
$e = [1, 2];
$t = &$e[0];
unset($t);
show($e);
$y = 1;
$inner = [&$y, 2];
unset($y);
show([$inner, $inner]);

// References in __serialize's data and in __sleep's properties.
class Data {
    public $one = 1;
    public $two;
    function __construct() { $this->two = &$this->one; }
    function __serialize(): array { $v = 'v'; return ['x' => &$v, 'y' => &$v, 'z' => &$this->one]; }
    function __unserialize(array $data): void { $data['x'] = 'set'; echo $data['y'], "\n"; }
}
show([new Data, 'after']);
unserialize(serialize(new Data));
class Sleepy {
    public $a = 'a';
    public $b;
    public $c = 'c';
    function __construct() { $this->b = &$this->a; }
    function __sleep() { return ['a', 'b', 'c']; }
}
show(new Sleepy);
$u = unserialize(serialize(new Sleepy));
$u->a = 'A';
echo $u->b, "\n";

// unserialize() of R: and r: as written by hand.
var_dump(unserialize('a:3:{i:0;s:1:"x";i:1;R:2;i:2;R:2;}'));
$u = unserialize('a:2:{i:0;a:1:{i:0;i:7;}i:1;R:3;}');
$u[1] = 8;
echo $u[0][0], "\n";
$u = unserialize('a:2:{i:0;O:8:"stdClass":0:{}i:1;R:2;}');
$u[1] = 'no object';
var_dump($u[0]);
var_dump(unserialize('a:2:{i:0;i:1;i:1;r:2;}'));
var_dump(unserialize('a:2:{i:0;O:8:"stdClass":0:{}i:1;r:2;}'));
var_dump(unserialize('a:1:{i:0;R:3;}'));
var_dump(unserialize('a:1:{i:0;R:0;}'));
var_dump(unserialize('a:2:{i:0;i:1;i:1;R:3;}'));
$u = unserialize('a:2:{i:0;O:8:"stdClass":1:{s:1:"p";i:1;}i:1;R:3;}');
$u[1] = 2;
var_dump($u[0]);
var_dump(unserialize('a:3:{i:0;O:8:"stdClass":0:{}i:1;r:2;i:2;R:3;}'));
var_dump(unserialize('a:2:{i:5;a:0:{}i:1;R:2;}'));
var_dump(unserialize('a:2:{i:0;s:1:"a";i:1;R:1;}'));
var_dump(unserialize('a:1:{i:0;R:1;}'));
var_dump(unserialize('R:1;'));
var_dump(unserialize('a:1:{R:1;i:1;}'));
var_dump(unserialize('a:1:{r:1;i:1;}'));
var_dump(unserialize('a:2:{i:0;i:1;i:1;R:+2;}'));
var_dump(unserialize('a:2:{i:0;i:1;i:1;R:2}'));
var_dump(unserialize('a:2:{s:1:"k";i:1;i:1;R:x;}'));
var_dump(unserialize('a:2:{s:1:"k";i:1;i:1;r:1;}'));
// A string that only looks like one.
var_dump(unserialize('a:2:{i:0;s:4:"R:1;";i:1;s:2:"R:";}'));
