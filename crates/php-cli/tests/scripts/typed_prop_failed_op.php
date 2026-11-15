<?php
// A typed property whose compound assignment's operation throws
// (`$o->f /= 0`) is given the operation's undefined result as its type
// takes it in weak mode: 0 where int is taken (mixed too), 0.0 where
// float is; a string or bool property, an untyped one, a hooked, readonly
// or not settable one keeps its value (#572).

function report(Throwable $e): void {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

class C {
    public float $f = 3.0;
    public int $i = 6;
    public ?float $nf = 2.0;
    public ?int $ni = 5;
    public mixed $m = 5;
    public string $s = "6";
    public bool $b = true;
    public int|string $is = 6;
    public float|string $fs = 6.5;
    public int|float $if = 2.5;
    public false|int $fi = 4;
    public string|bool $sb = "x";
    public array $a = [1];
    public $u = 9;
}

$c = new C();
foreach (["f", "i", "nf", "ni", "m", "s", "b", "is", "fs", "if", "fi", "sb", "u"] as $p) {
    try { $c->$p /= 0; } catch (Throwable $e) { report($e); }
    echo "$p: ";
    var_dump($c->$p);
}
$c->f = 3.0;
try { $c->f %= 0; } catch (Throwable $e) { report($e); }
var_dump($c->f);
$c->i = 6;
try { $c->i <<= -1; } catch (Throwable $e) { report($e); }
var_dump($c->i);
$c->i = 6;
try { $c->i >>= -2; } catch (Throwable $e) { report($e); }
var_dump($c->i);
$c->i = 6;
try { $c->i **= []; } catch (Throwable $e) { report($e); }
var_dump($c->i);
$c->i = 6;
try { $c->i += new stdClass(); } catch (Throwable $e) { report($e); }
var_dump($c->i);
$c->ni = 5;
try { $c->ni -= []; } catch (Throwable $e) { report($e); }
var_dump($c->ni);
$c->ni = 5;
try { $c->ni .= new stdClass(); } catch (Throwable $e) { report($e); }
var_dump($c->ni);
$c->ni = 5;
try { $c->ni |= new stdClass(); } catch (Throwable $e) { report($e); }
var_dump($c->ni);
try { $c->a += 1; } catch (Throwable $e) { report($e); }
var_dump($c->a);

// A failed type check of the result is not this: the property keeps its value.
$c->f = 3.0;
try { $c->f .= "x"; } catch (Throwable $e) { report($e); }
var_dump($c->f);
$c->s = "ab";
try { $c->s .= new stdClass(); } catch (Throwable $e) { report($e); }
var_dump($c->s);

// Through a dynamic name, and through $this.
$c->i = 7;
$name = "i";
try { $c->$name /= 0; } catch (Throwable $e) { report($e); }
var_dump($c->i);

class T {
    public int $i = 7;
    public float $f = 1.5;
    function run(): void {
        try { $this->i /= 0; } catch (Throwable $e) { report($e); }
        var_dump($this->i);
        try { $this->f %= 0; } catch (Throwable $e) { report($e); }
        var_dump($this->f);
    }
}
(new T())->run();

// A loop: the cached property write.
$t = new T();
for ($k = 0; $k < 3; $k++) {
    $t->i = 10 + $k;
    try { $t->i /= $k - 1; } catch (Throwable $e) { report($e); }
    var_dump($t->i);
}

// Asymmetric visibility: settable here or not.
class A {
    public private(set) int $p = 4;
    public protected(set) float $q = 2.0;
    function run(): void {
        try { $this->p /= 0; } catch (Throwable $e) { report($e); }
        var_dump($this->p);
        try { $this->q %= 0; } catch (Throwable $e) { report($e); }
        var_dump($this->q);
    }
}
(new A())->run();
$a = new A();
try { $a->p /= 0; } catch (Throwable $e) { report($e); }
var_dump($a->p);

// Readonly and hooked properties keep theirs.
class R {
    public readonly int $r;
    function __construct() { $this->r = 4; }
    function run(): void {
        try { $this->r /= 0; } catch (Throwable $e) { report($e); }
        var_dump($this->r);
    }
}
(new R())->run();
class H {
    public int $h = 4 { set(int $v) { echo "set $v\n"; $this->h = $v; } }
    public int $g = 3 { get => $this->g; }
}
$h = new H();
try { $h->h /= 0; } catch (Throwable $e) { report($e); }
var_dump($h->h);
try { $h->g /= 0; } catch (Throwable $e) { report($e); }
var_dump($h->g);

// Plain variables and array elements keep theirs.
$x = 7.0;
try { $x /= 0; } catch (Throwable $e) { report($e); }
var_dump($x);
$arr = ["k" => 7.0];
try { $arr["k"] /= 0; } catch (Throwable $e) { report($e); }
var_dump($arr);

// The old value is let go: its destructor runs.
class D { function __destruct() { echo "destructed\n"; } }
$c->m = new D();
try { $c->m /= 1; } catch (Throwable $e) { report($e); }
var_dump($c->m);
