<?php
// Quiet reads (`??`, isset, empty) of `$this->...` in an instance method read
// the frame's $this in place: the same answers, no warnings, magic methods
// called on the same object, destructors at the same statements, and `$this`
// outside object context still an error.

class Node {
    public $name;
    public function __construct($name) { $this->name = $name; }
    public function __destruct() { echo "destruct {$this->name}\n"; }
}

class Magic {
    private $hidden = ['k' => 'v'];
    public function __get($n) { echo "__get($n)\n"; return $n === 'arr' ? ['x' => 1] : null; }
    public function __isset($n) { echo "__isset($n)\n"; return $n === 'arr'; }
    public function read() {
        var_dump($this->arr['x'] ?? 'none');
        var_dump($this->nope ?? 'none');
        var_dump(isset($this->arr['x']));
        var_dump(isset($this->nope['x']));
        var_dump(empty($this->arr['x']));
        var_dump($this->hidden['k'] ?? 'none');
    }
}

class Holder {
    public $map = ['a' => 1, 'b' => null, 'c' => ['d' => 2]];
    public ?Holder $next = null;
    public int $typed;
    public $node;
    private $priv = 'p';

    public function reads($k) {
        var_dump($this->map[$k] ?? 'default');
        var_dump(isset($this->map[$k]));
        var_dump(empty($this->map[$k]));
        var_dump($this->map['c']['d'] ?? 'default');
        var_dump($this->map['c']['zz'] ?? 'default');
        var_dump($this->missing ?? 'no prop');
        var_dump(isset($this->missing['x']));
        var_dump($this->typed ?? 'uninitialized');
        var_dump(isset($this->typed));
        var_dump($this->next->map['a'] ?? 'no next');
        var_dump($this->priv ?? 'none');
        $name = 'map';
        var_dump($this->$name['a'] ?? 'dyn none');
        $name = 'zzz';
        var_dump($this->$name ?? 'dyn none');
    }

    public function chain() {
        $this->next = new Holder();
        $this->next->map['a'] = 'next a';
        var_dump($this->next->map['a'] ?? 'no');
        var_dump(isset($this->next->next->map));
        $this->next = null;
    }

    public function destructs() {
        $this->node = new Node('first');
        echo "before\n";
        $x = $this->node ?? null;
        echo "read\n";
        $this->node = null;
        echo "cleared, still held\n";
        $x = null;
        echo "released\n";
        $this->node = new Node('second');
        var_dump(isset($this->node->name));
        $this->node = null;
        echo "after second\n";
    }

    public function unset_and_read() {
        unset($this->map);
        var_dump($this->map['a'] ?? 'unset');
        var_dump(isset($this->map));
        $this->map = [1];
        var_dump($this->map[0] ?? 'gone');
    }

    public static function stat() {
        return function () { return $this->map['a'] ?? 'unbound default'; };
    }
}

$h = new Holder();
$h->reads('a');
$h->reads('b');
$h->reads('zz');
$h->chain();
$h->destructs();
$h->unset_and_read();
(new Magic())->read();

// A closure made in a method reads its bound $this; one rebound reads the new one.
class Box {
    public $v = ['k' => 'box'];
    public function getter() { return function () { return $this->v['k'] ?? 'none'; }; }
}
$g = (new Box())->getter();
var_dump($g());
$other = new Box();
$other->v = ['k' => 'other'];
var_dump(Closure::bind($g, $other, Box::class)());
var_dump(Closure::bind($g, new Box(), Box::class)());

// No $this: an error, not a quiet null.
$f = Holder::stat();
try {
    var_dump($f());
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
// A fresh $this every call in a loop: nothing kept between calls.
$sum = 0;
for ($i = 0; $i < 3; $i++) {
    $o = new Holder();
    $o->map['a'] = $i;
    $sum += (function () { return $this->map['a'] ?? 0; })->call($o);
}
var_dump($sum);
echo "end\n";
