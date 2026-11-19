<?php
// A call's frame pushed and left by the interpreter loop's call arms:
// the caller's resume point, a slot reused by another function at the
// same depth, a depth no frame reached before (a new slot), the
// arguments bound from the numbers the arm has in hand (as many as the
// parameters, fewer with defaults, more for func_get_args), and the
// registers a return empties (the variables, and the temporaries of
// code with too many of them to free each where it dies).

class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "~{$this->n} "; }
}

class P {
    public int $calls = 0;
    public function __construct(public string $tag = 'p', int ...$rest) {
        echo " P({$this->tag}", $rest ? ',' . implode(',', $rest) : '', ")";
    }
    public function exact($a, $b) { $this->calls++; return "$a$b"; }
    public function fewer($a, $b = 'B', $c = 'C') { $this->calls++; return "$a$b$c"; }
    public function more($a) { $this->calls++; return $a . '+' . implode('|', array_slice(func_get_args(), 1)); }
    public function onThis($x) { return $this->exact($x, 1) . $this->fewer($x) . $this->more($x, 2, 3); }
    public function locals($x) {
        // A variable not passed, read before it is set: undefined in
        // every frame, whatever the slot's last frame left there.
        $seen = isset($late) ? 'set' : 'unset';
        $late = $x;
        $d = new D("m$x");
        return $seen . ':' . $late;
    }
    public static function make($t = 'made', ...$more) { return static::class . "/$t/" . count($more); }
    public function depth($n) { return $n === 0 ? self::class : $this->depth($n - 1) . $n; }
}

class Q extends P {
    public function fewer($a, $b = 'b', $c = 'c') { return 'Q' . parent::fewer($a, $b, $c); }
}

function plain($a, $b = 2) { return $a * 10 + $b; }
function other($x) { $y = isset($z) ? 'z' : 'noz'; $z = 1; return $y . $x; }
function deep($n) { return $n === 0 ? 0 : 1 + deep($n - 1); }
function many(...$v) { return count($v); }

// More temporaries than the compiler frees one by one (live.rs MAX_TEMPS):
// at its return the frame's temporaries are emptied too, so the object
// read through a temporary goes before the caller goes on.
function wide($a) {
    $n = many($a + 1, $a + 2, $a + 3, $a + 4, $a + 5, $a + 6, $a + 7, $a + 8, $a + 9, $a + 10,
        $a + 11, $a + 12, $a + 13, $a + 14, $a + 15, $a + 16, $a + 17, $a + 18, $a + 19, $a + 20,
        $a + 21, $a + 22, $a + 23, $a + 24, $a + 25, $a + 26, $a + 27, $a + 28, $a + 29, $a + 30,
        $a + 31, $a + 32, $a + 33, $a + 34, $a + 35, $a + 36, $a + 37, $a + 38, $a + 39, $a + 40,
        $a + 41, $a + 42, $a + 43, $a + 44, $a + 45, $a + 46, $a + 47, $a + 48, $a + 49, $a + 50,
        $a + 51, $a + 52, $a + 53, $a + 54, $a + 55, $a + 56, $a + 57, $a + 58, $a + 59, $a + 60,
        $a + 61, $a + 62, $a + 63, $a + 64, $a + 65, $a + 66, $a + 67, $a + 68, $a + 69, $a + 70);
    $v = (new D("w$a"))->n;
    return "$n:$v";
}

// A function whose temporaries are each freed where they die, and which
// has no variable left at its return: the object goes at its last use.
function narrow($a) {
    return (new D("n$a"))->n;
}

$p = new P;
$q = new Q('q', 7, 8);
$f = function ($a, $b = 'fb') use ($p) { return "$a$b" . $p->calls; };
$g = fn($x, ...$r) => $x . count($r);

for ($i = 0; $i < 3; $i++) {
    echo "\n[$i] ";
    // Methods: $o->m() and $this->m(), each argument count.
    echo $p->exact('a', 'b'), ' ', $p->fewer('a'), ' ', $p->fewer('a', 'x'), ' ', $p->more('a', 1, 2), ' ';
    echo $p->onThis($i), ' ', $q->fewer('z'), ' ', $q->onThis('q'), "\n";
    // The same depth, one function after another (the slot's code swapped),
    // nested in one expression: each caller goes on where it called.
    echo plain(1), ' ', plain(2, 3), ' ', other('o'), ' ', plain(plain(1), plain(0, 1)), ' ', $p->locals($i), ' ', other('p'), "\n";
    // Static calls and constructors, defaults and extra arguments.
    echo P::make(), ' ', Q::make('t', 1, 2), ' ';
    $n = new P('n' . $i);
    $m = new Q(...[$i, 4, 5]);
    echo $n->tag, ' ', $m->tag, "\n";
    // Closures called as values.
    echo $f('c'), ' ', $f('c', 'd'), ' ', $g('g'), ' ', $g('g', 1, 2, 3), "\n";
    // Deeper than any frame so far (new slots), then back at every depth.
    echo deep(40 + $i * 30), ' ', $p->depth(5 + $i), ' ', deep(3), "\n";
    // Locals and temporaries emptied at the return.
    echo wide($i), " after-wide ";
    echo narrow($i), " after-narrow\n";
    unset($n, $m);
}
echo "\ncalls {$p->calls}\n";
