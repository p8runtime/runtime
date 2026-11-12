<?php
// `$o->m($a, 1, $b)` sends its plain variables after it finds the method
// (INIT_METHOD_CALL, then one send per argument, in order). What runs
// between the two, and what each send sees, as PHP has it.

function quiet($no, $msg) { echo "warning: $msg\n"; return true; }
set_error_handler('quiet');

class T {
    public $tag;
    function __construct($tag = 'T') { $this->tag = $tag; }
    function name($x) { return $this->tag . ' ' . var_export($x, true); }
    function two($a, $b) { return $this->tag . ' ' . var_export($a, true) . ' ' . var_export($b, true); }
    function ref(&$x) { $x .= '+' . $this->tag; return $this->tag; }
    function refval(&$a, $b) { $a .= 'a'; $b .= 'b'; return "$a $b"; }
    function valref($a, &$b) { $a .= 'a'; $b .= 'b'; return "$a $b"; }
}

// 1. The handler of "Undefined variable" runs at that variable's send:
// after the method is found, after the sends before it, before the sends
// after it.
function sees_later() {
    global $late;
    $o = new T;
    $late = 'before';
    $r = $o->two($undefined, $late);
    $late = 'before';
    return $r . ' | ' . $o->two($late, $undefined);
}
set_error_handler(function ($no, $msg) { $GLOBALS['late'] = 'after'; echo "warning: $msg\n"; return true; });
for ($i = 0; $i < 3; $i++) {
    echo sees_later(), "\n";
}

// 2. A handler that throws: the call does not run, the exception is the
// caller's, and the variables sent before it stay as the sends left them.
function thrown() {
    $o = new T;
    $v = 'v';
    try {
        return $o->refval($v, $undefined);
    } catch (ErrorException $e) {
        return 'caught ' . $e->getMessage() . ' ' . var_export($v, true);
    }
}
set_error_handler(function ($no, $msg) { throw new ErrorException($msg); });
for ($i = 0; $i < 3; $i++) {
    echo thrown(), "\n";
}

// 3. A handler that unsets the receiver's variable, or puts another object
// in it: this call runs on the object it found its method on.
function gone() {
    global $recv;
    $recv = new T('first');
    $r = $recv->name($undefined);
    return $r . ' ' . var_export(isset($recv), true);
}
set_error_handler(function ($no, $msg) { unset($GLOBALS['recv']); echo "warning: $msg\n"; return true; });
for ($i = 0; $i < 2; $i++) {
    echo gone(), "\n";
}
class U { function name($x) { return 'U::name'; } }
function swapped() {
    global $recv;
    $recv = new T('first');
    $r = $recv->name($undefined);
    return $r . ' ' . get_class($recv);
}
set_error_handler(function ($no, $msg) { $GLOBALS['recv'] = new U; echo "warning: $msg\n"; return true; });
for ($i = 0; $i < 2; $i++) {
    echo swapped(), "\n";
}

// 4. (A handler that suspends a fiber: scripts_later/method_call_handler_fiber.)
set_error_handler('quiet');

// 5. The same variable twice, by reference and by value, in both orders.
function twice() {
    $o = new T;
    $x = 'x';
    $r = $o->refval($x, $x);
    $y = 'y';
    $s = $o->valref($y, $y);
    return "$r $x | $s $y";
}
for ($i = 0; $i < 3; $i++) {
    echo twice(), "\n";
}

// 6. One site, two classes whose methods take the other argument by
// reference.
class RV { function m(&$a, $b) { $a .= '1'; $b .= '1'; return 'RV'; } }
class VR { function m($a, &$b) { $a .= '2'; $b .= '2'; return 'VR'; } }
function alternate($objects) {
    $a = 'a';
    $b = 'b';
    $out = '';
    foreach ($objects as $o) {
        $out .= $o->m($a, $b) . " $a $b, ";
    }
    return $out;
}
$rv = new RV;
$vr = new VR;
echo alternate([$rv, $vr, $rv, $vr, $vr, $rv, $rv]), "\n";
echo alternate([$vr, $rv, $vr]), "\n";

// 7. Named arguments and a spread are sent the general way.
// (What a spread array's own element shows after it went to a by-reference
// parameter: scripts_later/method_call_spread_by_ref.)
class N { function m($a, &$b, $c = 'c') { $b .= "[$a$c]"; return "$a$b$c"; } }
function named() {
    $o = new N;
    $x = 'x';
    $y = 'y';
    $r = $o->m(b: $y, a: $x);
    $s = $o->m($x, c: $x, b: $y);
    return "$r $s $x $y";
}
function spread() {
    $o = new N;
    $x = 'x';
    $y = 'y';
    $list = ['p', 'q', 'r'];
    $r = $o->m(...$list);
    $s = $o->m($x, ...[$y, 'z']);
    $pair = ['b' => 'k', 'a' => 'j'];
    $t = $o->m(...$pair);
    return "$r $s $t $x $y";
}
for ($i = 0; $i < 2; $i++) {
    echo named(), ' | ', spread(), "\n";
}

// 8. A receiver that is itself a reference.
function by_reference() {
    $o = new T('ref');
    $r = &$o;
    $v = 'v';
    $a = $r->ref($v);
    $b = $o->refval($v, $v);
    return "$a $b $v";
}
function global_receiver() {
    global $recv;
    $v = 'v';
    return $recv->ref($v) . " $v";
}
function static_receiver() {
    static $o = null;
    $o ??= new T('static');
    $v = 'v';
    return $o->valref($v, $v) . " $v";
}
function each_by_reference() {
    $list = [new T('one'), new T('two')];
    $v = 'v';
    $out = '';
    foreach ($list as &$o) {
        $out .= $o->ref($v) . ' ';
    }
    return $out . $v;
}
$recv = new T('global');
for ($i = 0; $i < 2; $i++) {
    echo by_reference(), ' | ', global_receiver(), ' | ', static_receiver(), ' | ', each_by_reference(), "\n";
}

// 9. A lazy object as the receiver: its initializer runs when the method
// first reads a property, with the arguments sent already.
class L {
    public $p = 'declared';
    function m(&$x, $y) { $x .= '+' . $this->p; return "$y"; }
}
function lazy($make) {
    global $shared;
    $shared = 'shared';
    $v = 'v';
    $class = new ReflectionClass('L');
    $o = $make($class);
    $r = $o->m($v, $shared);
    $s = $o->m($v, $shared);
    return "$r $s $v $shared";
}
echo lazy(fn ($c) => $c->newLazyGhost(function ($o) { echo "ghost made\n"; $GLOBALS['shared'] = 'changed'; $o->p = 'ghost'; })), "\n";
echo lazy(fn ($c) => $c->newLazyProxy(function ($o) { echo "proxy made\n"; $GLOBALS['shared'] = 'changed'; $real = new L; $real->p = 'real'; return $real; })), "\n";

// 10. A literal beside the variables: by value it is sent as it is, and the
// variables on either side of it as the method takes them.
// (A literal where the method takes a reference is PHP's Error:
// scripts_later/method_call_literal_by_ref.)
function literal($o) {
    $v = 'v';
    $w = 'w';
    $r = $o->refval($v, 'literal');
    $s = $o->valref(5, $w);
    $t = $o->two('first', $v) . ' ' . $o->two($w, null);
    return "$r | $s | $t | $v $w";
}
for ($i = 0; $i < 2; $i++) {
    echo literal(new T), "\n";
}

// 11. The receiver, and `$this`, as arguments.
class S {
    public $n = 0;
    function take($o) { $o->n++; return $o === $this ? 'same' : 'other'; }
    function replace(&$o) { $o = 'replaced'; return get_class($this); }
    function pass() { return $this->take($this); }
    function give($other) { return $other->take($this); }
}
function receiver_as_argument() {
    $o = new S;
    $a = $o->take($o);
    $b = $o->pass();
    $c = $o->give(new S);
    $n = $o->n;
    $d = $o->replace($o);
    return "$a $b $c $n $d $o";
}
for ($i = 0; $i < 2; $i++) {
    echo receiver_as_argument(), "\n";
}

// 12. __call and __invoke receivers.
class C {
    function __call($name, $args) { $args[0] .= '?'; return "$name(" . implode(',', $args) . ')'; }
    static function __callStatic($name, $args) { return "static $name"; }
    function __invoke(&$x, $y) { $x .= $y; return 'invoked'; }
}
function magic() {
    $o = new C;
    $v = 'v';
    $a = $o->anything($v, 1);
    $b = $o->__invoke($v, 2);
    $c = $o($v, 3);
    return "$a $b $c $v";
}
for ($i = 0; $i < 2; $i++) {
    echo magic(), "\n";
}

// 13. `?->`: no receiver, no method looked for, nothing sent.
function nullsafe($o) {
    $v = 'v';
    $r = $o?->ref($v);
    $s = $o?->two($undefined, $v);
    return var_export($r, true) . ' ' . var_export($s, true) . " $v";
}
foreach ([null, new T('safe'), null, new T('safe')] as $o) {
    echo nullsafe($o), "\n";
}

// 14. A generator suspended around the call, and a fiber suspended in it.
class G {
    function add(&$x, $y) { $x .= $y; return strlen($x); }
    function pause(&$x, $y) { $x .= Fiber::suspend("$x$y"); return 'paused'; }
}
function steps($o) {
    $x = 'x';
    while (true) {
        $got = yield $o->add($x, '.');
        if ($got === null) {
            return $x;
        }
        $x .= $got;
    }
}
$gen = steps(new G);
echo $gen->current(), ' ', $gen->send('a'), ' ', $gen->send('b'), ' ';
$gen->next();
echo $gen->getReturn(), "\n";
for ($i = 0; $i < 2; $i++) {
    $fiber = new Fiber(function ($o) {
        $v = 'v';
        $w = 'w';
        $r = $o->pause($v, $w);
        return "$r $v $w";
    });
    echo $fiber->start(new G), ' ';
    $fiber->resume("<$i>");
    echo $fiber->getReturn(), "\n";
}

// 15. A receiver that lives only in a temporary: when its destructor runs.
class D {
    public $tag;
    function __construct($tag) { $this->tag = $tag; }
    function m($x) { echo "m {$this->tag} ", var_export($x, true), "\n"; return 'returned'; }
    function r(&$x) { $x = "by {$this->tag}"; return 'returned'; }
    function __destruct() { echo "destructed {$this->tag}\n"; }
}
function temporary() {
    $v = 'v';
    echo (new D('a'))->m($v), "\n";
    echo (new D('b'))->r($v), " $v\n";
    echo (new D('c'))->m($undefined), "\n";
    $r = (new D('d'))->m($v);
    echo "after d\n";
    try {
        (new D('e'))->absent($v);
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    echo "after e\n";
}
temporary();
temporary();
set_error_handler(function ($no, $msg) { throw new ErrorException($msg); });
function temporary_thrown() {
    try {
        echo (new D('f'))->m($undefined), "\n";
    } catch (ErrorException $e) {
        echo 'caught ', $e->getMessage(), "\n";
    }
    echo "after f\n";
}
temporary_thrown();
temporary_thrown();
echo "done\n";
