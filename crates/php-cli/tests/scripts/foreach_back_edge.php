<?php
// A foreach's back edge, which is its next step (the jump and the step
// one instruction): what each step can find there, the body having
// changed things since the step before it.

class D {
    public function __construct(public string $name) {}
    public function __destruct() { echo "destruct {$this->name}\n"; }
}

class Loud {
    public function __construct(public string $name) {}
    public function __destruct() {
        echo "destruct {$this->name}\n";
        throw new Exception("from {$this->name}");
    }
}

// The body leaves the loop variable the last handle of an object: the
// step lets it go, its destructor runs there, before the next body.
foreach ([1, 2, 3] as $v) {
    echo "body $v\n";
    $v = new D("value $v");
}
foreach (['a' => 1, 'b' => 2] as $k => $v) {
    echo "body $k $v\n";
    $v = new D("value $k");
}
echo "after\n";

// A destructor the step runs throws: the exception is the step's, from
// the foreach line, and leaves the loop.
function thrown(array $a, bool $key): void {
    try {
        if ($key) {
            foreach ($a as $k => $v) {
                echo "body $k\n";
                $v = new Loud("keyed $k");
            }
        } else {
            foreach ($a as $v) {
                echo "body $v\n";
                $v = new Loud("value $v");
            }
        }
    } catch (Exception $e) {
        echo get_class($e), ': ', $e->getMessage(), ' at line ', $e->getLine(), ', called from line ', $e->getTrace()[0]['line'] ?? '-', "\n";
    }
}
thrown([1, 2, 3], false);
thrown(['x' => 1, 'y' => 2], true);

// The body makes the loop variable, then the key, a reference: the
// next steps write through it.
$a = ['p' => 1, 'q' => 2, 'r' => 3, 's' => 4];
$held = null;
foreach ($a as $k => $v) {
    if ($k === 'q') {
        $held = &$v;
    }
    echo "$k=$v held=", json_encode($held), "\n";
}
unset($held, $v);
$keyheld = null;
foreach ($a as $k => $v) {
    if ($k === 'p') {
        $keyheld = &$k;
    }
    echo "$k=$v key held=", json_encode($keyheld), "\n";
}
unset($keyheld, $k);
$held = null;
foreach ([10, 20, 30] as $v) {
    if ($v === 10) {
        $held = &$v;
    }
    echo "$v held=", json_encode($held), "\n";
}
unset($held, $v);

// The body changes the array the loop runs over: by value, the loop
// keeps the elements it started with.
$a = [1, 2, 3];
foreach ($a as $k => $v) {
    $a[] = $v * 10;
    unset($a[$k + 1]);
    $r = &$a[0];
    $r = 'changed';
    unset($r);
    echo "$k=$v ";
}
echo json_encode($a), "\n";
$a = [1, 2, 3];
foreach ($a as $v) {
    $a = 'gone';
    echo "$v ";
}
echo json_encode($a), "\n";

// An element made a reference by the body (a by-reference loop's own
// copy, then a by-value loop over it).
$a = ['m' => 'one', 'n' => 'two', 'o' => 'three'];
$x = 'x';
foreach ($a as $k => $v) {
    if ($k === 'm') {
        $a['n'] = &$x;
    }
    echo "$k=$v ";
}
echo json_encode($a), "\n";
foreach ($a as $k => $v) {
    echo "$k=$v ";
}
echo "\n";

// continue (a jump to the step from the middle of the body), break,
// and nested loops, the inner one's back edge inside the outer's body.
foreach (['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4] as $k => $v) {
    if ($v % 2) {
        continue;
    }
    foreach ([1, 2, 3] as $w) {
        if ($w === 2) {
            continue;
        }
        if ($v === 4 && $w === 3) {
            continue 2;
        }
        echo "$k$w ";
    }
    echo "| ";
}
echo "\n";
foreach ([1, 2, 3] as $v) {
    foreach (['x' => 1, 'y' => 2] as $k => $w) {
        if ($v === 2 && $k === 'y') {
            break 2;
        }
        echo "$v$k ";
    }
}
echo "\n";
$n = 0;
foreach (range(1, 100) as $v) {
}
foreach (range(1, 100) as $k => $v) {
    $n += $k;
}
echo "$v $k $n\n";

// Not an array: an Iterator, a generator, an object's properties; each
// step the body's back edge is the object's.
function gen() {
    yield 'g' => 1;
    yield 'h' => 2;
    yield 'i' => 3;
}
foreach (gen() as $k => $v) {
    if ($k === 'h') {
        continue;
    }
    echo "$k=$v ";
}
foreach (gen() as $v) {
    echo "$v ";
}
foreach (new ArrayIterator(['i' => 1, 'j' => 2]) as $k => $v) {
    echo "$k=$v ";
}
$o = new stdClass;
$o->a = 1;
$o->b = 2;
foreach ($o as $k => $v) {
    $o->$k = $v * 10;
    echo "$k=$v ";
}
echo json_encode($o), ' ';
echo "\n";

// An array, then an object, through one loop's variable.
foreach ([[1, 2], new ArrayIterator([3, 4]), ['k' => 5]] as $it) {
    foreach ($it as $k => $v) {
        echo "$k=$v ";
    }
}
echo "\nend\n";
