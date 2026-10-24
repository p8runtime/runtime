<?php
class A {
    private $data = ['a' => 1];
    private $secret = 's';
    public $plain = 'p';
    public $removed = 'r';
    function __get($n) { echo "get $n\n"; return $this->data[$n] ?? null; }
    function __set($n, $v) { echo "set $n\n"; $this->data[$n] = $v; }
    function __isset($n) { echo "isset $n\n"; return isset($this->data[$n]); }
    function __unset($n) { echo "unset $n\n"; unset($this->data[$n]); }
}
$a = new A;
var_dump($a->a, $a->missing, $a->plain);
$a->b = 2;
var_dump($a->b);
var_dump(isset($a->a), isset($a->zz), isset($a->plain));
var_dump($a->a ?? 'default', $a->zz ?? 'default');
$a->a += 10;
var_dump($a->a);
$a->a++;
var_dump($a->a);
unset($a->a);
var_dump(isset($a->a));
var_dump($a->secret);
$a->secret = 'x';
unset($a->removed);
var_dump($a->removed);
$a->arr[] = 1;
var_dump(empty($a->b));

class G { function __get($n) { echo "G get $n\n"; return $this->$n; } }
$g = new G;
var_dump($g->x);

class OnlyGet { function __get($n) { return 'v'; } }
$o = new OnlyGet;
var_dump(isset($o->x), $o->x ?? 'dflt');
$o->y = 5;
var_dump($o->y);
