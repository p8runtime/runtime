<?php
// Every case one foreach-by-value step has, for a list and for a hashed
// array, with and without the key: what the position holds (an entry, a
// hole, the end), what the element is (a value, a reference), and what
// the loop's variables held before the step (a scalar, a shared value,
// the last handle of a value, a reference).

class D {
    public function __construct(public string $name) {}
    public function __destruct() { echo "destruct {$this->name}\n"; }
}

function show(string $label, array $a): void {
    echo "$label:";
    foreach ($a as $v) {
        echo ' ', json_encode($v);
    }
    echo ' |';
    foreach ($a as $k => $v) {
        echo ' ', json_encode($k), '=>', json_encode($v);
    }
    echo "\n";
}

// What the position holds. Holes first, last, alone, in runs; every
// entry unset; nothing at all.
show('empty', []);
show('list', [1, 'two', 3.5, null, true, [4]]);
show('hash', ['a' => 1, 'b' => 'two', 7 => 3.5, -3 => null, '' => true, 'n' => [4]]);
$l = [10, 11, 12, 13, 14, 15, 16];
unset($l[0]);
show('list, first unset', $l);
unset($l[3], $l[4]);
show('list, a run unset', $l);
unset($l[6]);
show('list, last unset', $l);
unset($l[1], $l[2], $l[5]);
show('list, all unset', $l);
$l[] = 'next';
show('list, pushed after', $l);
$h = ['a' => 10, 'b' => 11, 5 => 12, 'c' => 13, 'd' => 14, 6 => 15, 'e' => 16];
unset($h['a']);
show('hash, first unset', $h);
unset($h['c'], $h['d']);
show('hash, a run unset', $h);
unset($h['e']);
show('hash, last unset', $h);
unset($h['b'], $h[5], $h[6]);
show('hash, all unset', $h);
$h['z'] = 'next';
$h[] = 'pushed';
show('hash, set after', $h);
show('sparse', [3 => 'three', 1 => 'one', 2 => 'two']);
show('from one', [1 => 'a', 2 => 'b', 4 => 'c']);

// The key is the array's own, and stays it: the loop's copy changed in
// the body leaves the array's keys and values as they were.
$words = ['alpha' => str_repeat('a', 3), 'beta' => str_repeat('b', 3), 12 => str_repeat('c', 3)];
foreach ($words as $k => $v) {
    $k .= '!';
    $v .= '?';
    echo "$k $v ";
}
echo "\n";
var_dump($words);
foreach ($words as $v) {
    $v .= '?';
}
foreach ($words as $k => $v) {
    $k = [$k];
    $v = [$v];
}
echo json_encode($words), ' ', json_encode($k), ' ', json_encode($v), "\n";

// The old value of the loop variable is its last handle: let go at the
// step, a destructor run there, before the next body (and with the key
// already the next one).
foreach ([1, 2, 3] as $v) {
    echo "body $v\n";
    $v = new D("made in body $v");
}
echo "after\n";
unset($v);
foreach (['x' => 1, 'y' => 2, 'z' => 3] as $k => $v) {
    echo "body $k $v\n";
    $v = new D("made in body $k");
}
echo "after\n";
unset($v);
foreach (['x' => 1, 'y' => 2] as $k => $v) {
    echo "body $k $v\n";
    $k = new D("key made in body $v");
}
echo "after\n";
unset($k);
foreach ([1, 2] as $k => $v) {
    echo "body $k $v\n";
    $k = new D("key of body $v");
    $v = new D("value of body $v");
}
echo "after\n";
unset($k, $v);

// The destructor runs at the step itself, before the body's first
// statement, whatever that statement is (arithmetic on locals and the
// echo of a constant free nothing, so nothing after the step would run
// it before them): the value's old value, the value's with a key beside
// it, the key's, and each through a reference the variable holds.
function at_the_step(): void {
    $n = 0;
    foreach ([1, 2, 3] as $v) {
        $n += $v;
        echo "value body\n";
        $v = new D("value $n");
    }
    echo "after\n";
    unset($v);
    $n = 0;
    foreach ([1, 2, 3] as $k => $v) {
        $n += $v;
        echo "keyed value body\n";
        $v = new D("keyed value $n");
    }
    echo "after\n";
    unset($v);
    $n = 0;
    foreach ([1, 2, 3] as $k => $v) {
        $n += $v;
        echo "key body\n";
        $k = new D("key $n");
    }
    echo "after\n";
    unset($k);
    $n = 0;
    $target = null;
    $r = &$target;
    foreach ([1, 2, 3] as $r) {
        $n += $r;
        echo "reference body\n";
        $target = new D("through the reference $n");
    }
    echo "after\n";
    unset($r);
    $n = 0;
    $kt = null;
    $rk = &$kt;
    foreach ([1, 2, 3] as $rk => $v) {
        $n += $v;
        echo "key reference body\n";
        $kt = new D("through the key's reference $n");
    }
    echo "after\n";
    unset($rk);
    $n = 0;
    $vt = null;
    $rv = &$vt;
    foreach ([1, 2, 3] as $k => $rv) {
        $n += $k;
        echo "keyed reference body\n";
        $vt = new D("through the keyed value's reference $n");
    }
    echo "after\n";
}
at_the_step();
echo "returned\n";

// The elements are objects: each is kept by the array while the loop
// variable moves on, and goes when the array does.
$objects = [new D('first'), 'k' => new D('second')];
foreach ($objects as $v) {
    echo "at {$v->name}\n";
}
foreach ($objects as $k => $v) {
    echo "at $k {$v->name}\n";
}
unset($v);
echo "loop variable gone\n";
unset($objects);
echo "array gone\n";

// The loop's variables hold references: every step writes through.
function through(): void {
    global $gv, $gk;
    foreach (['p' => 'one', 'q' => 'two'] as $gv) {
    }
    foreach (['p' => 'one', 'q' => 'two', 'r' => 'three'] as $gk => $gv) {
    }
}
through();
var_dump($gv, $gk);
$target = 'unset';
$v = &$target;
foreach ([5, 6, 7] as $v) {
    echo "$target ";
}
echo "\n";
unset($v);
$kt = 'unset';
$k = &$kt;
foreach (['m' => 5, 'n' => 6] as $k => $v) {
    echo "$kt=$v ";
}
echo "\n";
unset($k, $v);
// The value through a reference whose old value is its last handle, the
// key through a reference too.
$vt = new D('referenced value');
$kt = new D('referenced key');
$v = &$vt;
$k = &$kt;
foreach (['m' => 5] as $k => $v) {
    echo "body $kt $vt\n";
}
unset($k, $v);
// The value's old value its last handle, the key through a reference.
$kt = 'unset';
$k = &$kt;
foreach (['m' => 5, 'n' => 6] as $k => $v) {
    echo "body $kt $v\n";
    $v = new D("value at $kt");
}
echo "after $kt\n";
unset($k, $v);

// Elements that are references: the loop gets their values.
$one = 'one';
$two = ['two'];
$refs = [&$one, 'k' => &$two, 'plain'];
foreach ($refs as $v) {
    $v = 'changed';
}
foreach ($refs as $k => $v) {
    echo json_encode($k), '=>', json_encode($v), ' ';
    $v = 'changed';
}
echo "\n", json_encode([$one, $two, $refs]), "\n";

// The key and the value in one variable: the key is written last.
foreach (['a' => 'A', 'b' => 'B', 3 => 'C'] as $same => $same) {
    var_dump($same);
}
$st = 'unset';
$same = &$st;
foreach (['a' => 'A', 'b' => 'B'] as $same => $same) {
    var_dump($st);
}
unset($same);
foreach (['a' => 'A', 'b' => 'B'] as $same => $same) {
    $same = new D("same at $same");
}
echo "after\n";
unset($same);

// The loop walks the array it began with: the body's changes to the
// variable it came from do not move it, and a break leaves the next
// loop to begin again.
$grow = ['a' => 1, 'b' => 2];
foreach ($grow as $k => $v) {
    $grow[$k . $k] = $v * 10;
    unset($grow['b']);
    echo "$k=$v ";
}
echo json_encode($grow), "\n";
$again = [1, 2, 3, 4];
foreach ($again as $v) {
    if ($v === 2) {
        break;
    }
}
foreach ($again as $i => $v) {
    foreach ($again as $j => $u) {
        if ($u > $v) {
            continue 2;
        }
        echo "$i$j ";
    }
}
echo "\n";

// What is not an array's step: an object's properties, an Iterator, a
// generator.
$o = new stdClass;
$o->p = 1;
$o->q = 'two';
foreach ($o as $k => $v) {
    echo "$k=$v ";
}
foreach (new ArrayIterator(['i' => 1, 'j' => 2]) as $k => $v) {
    echo "$k=$v ";
}
function gen() {
    yield 'g' => 1;
    yield 'h' => 2;
}
foreach (gen() as $k => $v) {
    echo "$k=$v ";
}
foreach (gen() as $v) {
    echo "$v ";
}
echo "\n";

// A long walk: every position once, in order, sums right.
$big = [];
for ($i = 0; $i < 1000; $i++) {
    $big[] = $i;
    $big["s$i"] = "v$i";
}
for ($i = 0; $i < 2000; $i += 7) {
    unset($big[$i % 2 ? "s$i" : intdiv($i, 2)]);
}
$sum = 0;
$len = 0;
$count = 0;
foreach ($big as $k => $v) {
    $count++;
    if (is_int($v)) {
        $sum += $v + $k;
    } else {
        $len += strlen($k) + strlen($v);
    }
}
$count2 = 0;
foreach ($big as $v) {
    $count2++;
}
echo "$count $count2 $sum $len ", count($big), "\n";
echo "end\n";
