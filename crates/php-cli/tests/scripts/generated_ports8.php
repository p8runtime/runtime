<?php
// The eighth batch of loop ports (#375) and their cold cases: array writes
// through a by-reference variable (one lookup; an element bound to the
// variable's own cell), a property's path written when the property is
// bound by reference, path writes of a literal value, `$x::class` of a
// class name, reads of a string's offset and of a missing element through
// a reference, casts of objects to bool and array, `global` out of line,
// a variable sent to a function not resolved yet, a property read of an
// object a reference holds. Each case runs twice, so the second run goes
// through the caches the first filled. The expected output is PHP 8.6's.

function put(array &$a, $k, $v) { $a[$k] = $v; return is_array($a) ? count($a) : gettype($a); }
function put_used(array &$a, $k, $v) { return $a[$k] = $v; }

echo "-- writes through a by-reference variable\n";
for ($r = 0; $r < 2; $r++) {
    $a = ['x' => 0, 3 => 0];
    echo put($a, 3, 'three'), put($a, 'x', 'ex'), put($a, 9, 'nine'), put($a, 'new', [1]), put($a, '7', 'seven'), ' ', json_encode($a), "\n";
    $e = 1; $b = ['k' => 0, 2 => 0]; $b['k'] = &$e; $b[2] = &$e;
    put($b, 'k', 'via-k'); echo $e, ' '; put($b, 2, 'via-2'); echo $e, ' ', put_used($b, 2, 'used'), ' ', $e, "\n";
    // An element bound to its own array's variable: the variable becomes the value.
    $s = [0]; $s[0] = &$s; put($s, 0, 1); echo gettype($s), ' ', var_export($s, true), "\n";
    $s = ['k' => 0]; $s['k'] = &$s; echo var_export(put_used($s, 'k', 'v'), true), ' ', gettype($s), ' ', $s, "\n";
    $s = [0]; $s[0] = &$s; $s[0] = 2; echo gettype($s), ' ', $s, "\n";
    $s = ['k' => 0]; $s['k'] = &$s; $s['k'] = 'str'; echo gettype($s), ' ', $s, "\n";
    // Two keys, a literal value (#457).
    $s = [[0]]; $s[0][0] = &$s; $s[0][0] = 1; echo gettype($s), ' ', $s, "\n";
    $s = [[0]]; $s[0][0] = &$s; $one = 1; $s[0][0] = $one; echo gettype($s), ' ', $s, "\n";
}

echo "-- path writes of a literal value\n";
function mark(array &$a, $k) { $a['seen'][$k] = true; $a['n'][$k][] = 'lit'; $a['z'][$k] = null; }
for ($r = 0; $r < 2; $r++) {
    $a = [];
    mark($a, 'p'); mark($a, 'q'); mark($a, 'p'); mark($a, 5);
    echo json_encode($a), "\n";
    $p = ['a' => ['b' => 1]]; $q = &$p; $q['a']['c'] = 2.5; $q['x']['y'] = 'z'; $q['a']['b'] = false;
    echo json_encode($p), "\n";
    $n = null; $n['a']['b'] = 1; echo json_encode($n), "\n";
    $str = ['s' => 'text'];
    try { $str['s']['x'] = 1; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    $str = ['s' => 'text']; $str['s'][0] = 'T'; echo $str['s'], "\n";
}

echo "-- a property bound by reference, its path written\n";
class Hook {
    private $it = []; private $pr = [10 => 'a', 20 => 'b']; public $cb = [];
    public $self = [0];
    function apply() {
        $l = 0; $out = '';
        $this->it[$l] = $this->pr;
        do { $out .= current($this->it[$l]); } while (false !== next($this->it[$l]));
        unset($this->it[$l]);
        $this->it[$l + 1] = 'x';
        return $out . count($this->it);
    }
    function add($p, $k) { $this->cb[$p][$k] = $p * 10 + $k; next($this->cb[$p]); return count($this->cb[$p]); }
    function bind_self() { $this->self[0] = &$this->self; $this->self[0][0] = 'deep'; return gettype($this->self); }
}
for ($r = 0; $r < 2; $r++) {
    $h = new Hook;
    echo $h->apply(), ' ', $h->apply(), ' ', $h->add(1, 2), $h->add(1, 3), $h->add(2, 1), ' ', json_encode($h->cb), "\n";
    echo $h->bind_self(), "\n";
}

echo "-- ::class of a class name\n";
class Fac { static function make($x) { return "made $x"; } }
class Holder { public $cls = 'Fac'; function call($m, $x) { return $this->cls::$m($x); } }
for ($r = 0; $r < 2; $r++) {
    $ho = new Holder;
    echo $ho->call('make', 1), "\n";
    $ho->cls = 'fac'; echo $ho->call('make', 2), "\n";
    $ho->cls = 'Nope';
    try { $ho->call('make', 3); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    $ho->cls = 42;
    try { $ho->call('make', 4); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    $ho->cls = new Fac; echo $ho->call('make', 5), "\n";
    $c = 'stdClass'; $rc = &$c;
    try { echo $rc::class, "\n"; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    $c = new ArrayObject; echo $rc::class, "\n";
}

echo "-- reads through a reference\n";
function first(&$s, $i) { return $s[$i]; }
function quiet(&$r, $k) { return var_export(empty($r[$k]), true) . var_export($r[$k] ?? 'dflt', true); }
for ($r = 0; $r < 2; $r++) {
    $s = 'hello';
    echo first($s, 0), first($s, -1), first($s, 4), "\n";
    echo first($s, 10), "|\n";
    $arr = ['a' => 1, 'z' => 0, 'n' => null]; $x = 5; $arr['r'] = &$x;
    echo quiet($arr, 'a'), quiet($arr, 'z'), quiet($arr, 'n'), quiet($arr, 'missing'), quiet($arr, 'r'), "\n";
    $nul = null; $int = 3; $st = 'ab';
    echo quiet($nul, 'k'), quiet($int, 0), quiet($st, 1), quiet($st, 5), "\n";
    echo first($arr, 'r'), "\n";
}

echo "-- casts of objects\n";
class Props { public $a = 1; protected $b = 2; private $c = 3; public $u; }
class Kid extends Props { private $c = 'kid'; }
for ($r = 0; $r < 2; $r++) {
    $o = new Kid; $o->dyn = 'd';
    $arr = (array) $o;
    echo var_export((bool) $o, true), ' ', count($arr), ' ', json_encode(array_map(fn($k) => str_replace("\0", '0', $k), array_keys($arr))), "\n";
    $std = new stdClass; echo var_export((bool) $std, true), count((array) $std), "\n";
    $f = fn() => 1; $fa = (array) $f; echo count($fa), ' ', $fa[0] instanceof Closure ? 'closure' : 'other', "\n";
    $ao = new ArrayObject(['p' => 1, 'q' => 2]); echo json_encode((array) $ao), "\n";
    $ref = &$o; echo json_encode(array_values((array) $ref)), ' ', var_export((bool) $ref, true), "\n";
}

echo "-- global out of line\n";
$mainvar = 'main';
function readg() { global $mainvar, $later; $later = ($later ?? 0) + 1; return "$mainvar $later"; }
for ($r = 0; $r < 2; $r++) {
    global $mainvar, $fresh;
    echo readg(), ' ', var_export($fresh, true), "\n";
    $fresh = 'set';
}
echo $later, "\n";

echo "-- variables sent to a function not resolved yet\n";
if (true) {
    function late($x, &$y, $z = 'z') { $y = "$x-y"; return "$x$z"; }
}
for ($r = 0; $r < 2; $r++) {
    $v = 'v'; $w = null;
    echo late($v, $w), ' ', $w, "\n";
    echo late($undefined_var, $w), ' ', $w, "\n";
}

echo "-- a property read of an object a reference holds\n";
class Magic { public $declared = 'decl'; private $hidden = 'h'; function __get($n) { return "magic $n"; } }
class Plain { public $p = 'plain'; private $priv = 'x'; }
function readm() { global $gm; return $gm->declared . ' ' . $gm->undeclared . ' ' . $gm->hidden; }
$gm = new Magic;
for ($r = 0; $r < 2; $r++) {
    echo readm(), "\n";
    $pl = new Plain; $rp = &$pl; echo $rp->p, "\n";
    try { echo $rp->priv; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    echo $rp->nothing ?? 'none', "\n";
    unset($rp);
}
