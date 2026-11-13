<?php
// The last handle of an array goes (zend_array_destroy): every size of
// buffer (empty, the pooled 8 and 16, past them), lists and hashes, each
// element released in order; what a destructor sees and does while the
// array goes (throws, keeps its object alive, frees another array, makes
// new arrays that reuse the freed buffers); weak references and WeakMap
// entries cleared then; object ids reused; references held by the array
// outliving it; and a long run of frees leaving memory where it was.
class D {
    public $n;
    function __construct($n) { $this->n = $n; }
    function __destruct() { echo "~{$this->n}\n"; }
}

echo "-- every size, list and hash\n";
foreach ([0, 1, 7, 8, 9, 16, 17, 40] as $size) {
    $l = [];
    $h = [];
    for ($i = 0; $i < $size; $i++) { $l[] = "v$i"; $h["k$i"] = $i; }
    $l[] = new D("list$size");
    $h['last'] = new D("hash$size");
    $l = null;
    unset($h);
    echo "after $size\n";
}
$e = [];
$e = null;
echo "empty gone\n";

echo "-- a hash with holes\n";
$h = ['a' => new D('a'), 'b' => new D('b'), 'c' => new D('c'), 'd' => new D('d')];
unset($h['b']);
echo "b unset\n";
$h = null;
echo "after holes\n";

echo "-- a list with holes, turned into a hash\n";
$l = [new D('l0'), new D('l1'), new D('l2')];
unset($l[1]);
$l[7] = new D('l7');
$l['s'] = new D('ls');
$l = 0;
echo "after mixed\n";

echo "-- a destructor that throws while the array goes\n";
class T {
    public $n;
    function __construct($n) { $this->n = $n; }
    function __destruct() { echo "~T{$this->n}\n"; if ($this->n === 't1') throw new Exception("from {$this->n}"); }
}
try {
    $t = [new T('t0'), new T('t1'), new T('t2')];
    $t = null;
    echo "not reached\n";
} catch (Exception $ex) {
    echo "caught: ", $ex->getMessage(), "\n";
}
echo "after throw\n";

echo "-- a destructor keeps its object alive\n";
$kept = [];
class R {
    public $n;
    function __construct($n) { $this->n = $n; }
    function __destruct() { global $kept; echo "~R{$this->n}\n"; $kept[] = $this; }
}
$r = ['x' => new R('r0'), 'y' => new R('r1')];
$r = null;
echo "kept: ", count($kept), " ", $kept[0]->n, " ", $kept[1]->n, "\n";
$kept = null;
echo "after keep\n";

echo "-- a destructor frees another array and makes new ones\n";
$other = [new D('o0'), [new D('o1')]];
class F {
    function __destruct() {
        global $other, $made;
        echo "~F\n";
        $other = null;
        $made = [];
        for ($i = 0; $i < 20; $i++) { $made[] = [$i, "m$i", [$i]]; }
        echo "made ", count($made), " ", $made[19][1], "\n";
    }
}
$f = [1, new F, 'z' => [new D('f1')]];
$f = null;
echo "made after: ", count($made), " ", $made[5][2][0], "\n";
$made = null;

echo "-- weak references and WeakMap entries\n";
$o = new D('w0');
$wr = WeakReference::create($o);
$wm = new WeakMap;
$wm[$o] = 'value';
$held = ['o' => $o, 'n' => [1, 2, 3]];
$o = null;
echo "held: ", $wr->get() === null ? "cleared" : "alive", " ", count($wm), "\n";
$held = null;
echo "freed: ", $wr->get() === null ? "cleared" : "alive", " ", count($wm), "\n";

echo "-- object ids reused after the array goes\n";
$a = [new stdClass, new stdClass, new stdClass];
$ids = array_map('spl_object_id', $a);
$a = null;
$b = [new stdClass, new stdClass, new stdClass];
$again = array_map('spl_object_id', $b);
sort($ids);
sort($again);
echo $ids === $again ? "same ids\n" : "other ids\n";
$b = null;

echo "-- references held by an array outlive it\n";
$x = 1;
$y = [10];
$refs = [&$x, 'k' => &$y, [&$x]];
$refs = null;
$x++;
$y[] = 11;
echo "x=$x y=", implode(',', $y), "\n";
$p = new stdClass;
$p->list = [1, 2];
$ra = [&$p->list];
$ra = null;
$p->list[] = 3;
echo "p->list=", implode(',', $p->list), "\n";

echo "-- cycles through references, collected\n";
$c = [new D('c0')];
$c[] = &$c;
$c = null;
echo "collected: ", gc_collect_cycles() > 0 ? "some" : "none", "\n";

echo "-- many frees leave memory where it was\n";
function round_trip() {
    for ($i = 0; $i < 200; $i++) {
        $l = [$i, "s$i", [$i], ['k' => $i]];
        $h = ['a' => $l, 'b' => [], 'c' => range(0, 20)];
        $l = $h = null;
    }
}
round_trip();
$before = memory_get_usage();
for ($n = 0; $n < 20; $n++) { round_trip(); }
echo memory_get_usage() === $before ? "no growth\n" : "grew\n";
echo "end\n";
