<?php
// new with constructors and property writes through a warm cache: the
// constructor found from the new's class (a new among the arguments,
// another class, none), its frame and arguments, and writes that store at
// once next to ones that need their checks (types, readonly, set
// visibility, references).

class Pt {
    public static int $made = 0;
    public function __construct(public int $x = 0, public int $y = 0) { self::$made++; }
}
class Pair {
    public function __construct(public Pt $a, public Pt $b) {}
}
class NoCtor { public $v = 1; public array $list = []; }
class Counted {
    public int $n = 0;
    public function __construct(int ...$more) { $this->n = count($more); }
}

// Nested news among the arguments, of the same and other classes.
for ($i = 0; $i < 3; $i++) {
    $p = new Pair(new Pt($i, $i + 1), new Pt(new NoCtor() instanceof NoCtor ? 7 : 0));
    echo $p->a->x, ",", $p->a->y, " ", $p->b->x, ",", $p->b->y, "\n";
}
echo Pt::$made, "\n";
echo (new Counted(1, 2, 3))->n, " ", (new Counted())->n, "\n";
try { new Pair(new Pt()); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
try { new Pt("no"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$o = new NoCtor;
var_dump($o->v);

// A constructor that throws: the object is not destructed.
class Fails {
    public function __construct() { throw new Exception("ctor"); }
    public function __destruct() { echo "never\n"; }
}
for ($i = 0; $i < 2; $i++) {
    try { new Fails; } catch (Exception $e) { echo $e->getMessage(), "\n"; }
}

// Writes through $this, the cache warm after the first round.
class W {
    public int $i = 0;
    public ?string $s = null;
    public float $f = 0.0;
    public $any;
    public readonly int $ro;
    public private(set) int $ps = 0;
    public function __construct() { $this->ro = 1; }
    public function round($k) {
        $this->i = $k;            // int: stored at once
        $this->s = "k$k";         // string into ?string
        $this->f = $k;            // int into float: converted
        $this->any = [$k];
        $this->ps = $k;
        $this->i += 1;
        $this->i++;
        return [$this->i, $this->s, $this->f, $this->any, $this->ps];
    }
    public function bad() { $this->i = "x"; }
    public function ro() { $this->ro = 2; }
}
$w = new W;
for ($k = 0; $k < 3; $k++) {
    var_dump($w->round($k));
}
try { $w->bad(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $w->ro(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $w->ps = 5; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $w->i = 1.5; } catch (TypeError $e) { echo get_class($e), "\n"; }
$w->i = "42";
var_dump($w->i);

// A property that is a reference: a write goes through it, typed or not.
$ref = &$w->i;
for ($k = 0; $k < 2; $k++) { $w->i = 100 + $k; }
var_dump($ref);
$ref2 = &$w->any;
$w->any = "through";
var_dump($ref2);
try { $w->i = "nope"; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump($ref);
