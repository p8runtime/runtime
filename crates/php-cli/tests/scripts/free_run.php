<?php
// Frees one after another (Op::FreeRun): temporaries released together,
// each destructor at its own Free, in order, the rest freed after it.
class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "~{$this->n}\n"; }
    public function v(): int { return strlen($this->n); }
}
function pair(): array { return [new D('a1'), new D('a2')]; }
function mk(string $n): D { return new D($n); }

// Two objects read in one expression: both temporaries die at its end.
echo mk('t1')->v() + mk('t2')->v(), "\n";
// A foreach over a temporary array broken out of: the iterator and the array.
foreach (pair() as $k => $d) {
    echo "in $k\n";
    unset($d);
    break;
}
echo "after foreach\n";
// Nested reads that leave several temporaries at once.
$s = mk('n1')->n . mk('n2')->n . mk('n3')->n;
echo "$s\n";
// A destructor that reads a variable the run's later Frees do not touch.
$log = [];
class E {
    public function __construct(public int $i) {}
    public function __destruct() { global $log; $log[] = $this->i; }
}
function e(int $i): E { return new E($i); }
for ($j = 0; $j < 3; $j++) {
    $x = e($j)->i + e($j + 10)->i + e($j + 20)->i;
}
echo implode(',', $log), " $x\n";
// Two objects compared: their temporaries freed one after the other.
var_dump(mk('p') == mk('q'));
var_dump(mk('p1') instanceof D && mk('q1') instanceof D);
// A destructor that throws at the first Free of a run: the second
// object is still released (once), after the exception is raised.
class T {
    public function __construct(public string $n) {}
    public function __destruct() { echo "~{$this->n}\n"; if ($this->n === 'boom') throw new Exception('from ' . $this->n); }
}
function t(string $n): T { return new T($n); }
try {
    var_dump(t('boom') == t('quiet'));
} catch (Exception $e) {
    echo 'caught ', $e->getMessage(), "\n";
}
echo "end\n";
