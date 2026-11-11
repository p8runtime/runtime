<?php
// The fifth batch of loop ports (#375) and their cold cases: method calls
// (the site's cache: a second class, __call, natives, scopes), sends of a
// variable by reference (SendVar), a parameter's default (ArgDefault),
// static property writes (AssignStaticProp), a static call's by-reference
// test (StaticArgByRef), references to properties (PropRef). Each case
// runs twice, so the second run goes through the caches the first filled.
// No `@`; the expected output is PHP 8.6's.

class Noisy {
    function __construct(public $name) {}
    function __destruct() { echo "destruct {$this->name}\n"; }
}

class Base {
    function name() { return 'base'; }
    function who() { return static::class . ':' . $this->name(); }
    protected function prot() { return 'prot ' . static::class; }
    private function priv() { return 'priv'; }
    function callPriv() { return $this->priv(); }
    static function make() { return new static; }
}
class Left extends Base { function name() { return 'left'; } function viaParent() { return parent::name() . '+' . $this->prot(); } }
class Right extends Base { function name() { return 'right'; } }
class Magic {
    function __call($m, $args) { return "__call $m(" . implode(',', $args) . ")"; }
    static function __callStatic($m, $args) { return "__callStatic $m(" . implode(',', $args) . ")"; }
    function real() { return 'real'; }
}

class Files { function notEmpty() { return 'files'; } }
class Dirs { function notEmpty() { return 'dirs'; } }
class Source {
    function __construct(public $f = new Files, public $d = new Dirs) {}
    // One name, two classes, two calls: each call its own cache entry.
    function both() { return $this->f->notEmpty() . '+' . $this->d->notEmpty() . '+' . $this->f->notEmpty(); }
}

echo "-- method calls\n";
function call_name($o) { return $o->name(); }
function call_who($o) { return $o->who(); }
// A receiver bound by `global` (a reference).
$GLOBALS['global_left'] = new Left;
function via_global($x) { global $global_left; return $global_left->name() . ' ' . $global_left->viaParent() . ' ' . $global_left->who($x); }
for ($r = 0; $r < 2; $r++) {
    echo (new Source)->both(), ' ', via_global($r), "\n";
    foreach ([null, new Right] as $maybe) {
        var_dump($maybe?->name());
    }
    $alias = new Right;
    $other = &$alias;
    echo $other->name(), ' ', $alias->who(), "\n";
    // One static method name on two classes in one function.
    echo get_class(Left::make()), ' ', get_class(Right::make()), ' ', get_class(Base::make()), "\n";
    // One site, several classes in turn.
    foreach ([new Left, new Right, new Base, new Left, new Right] as $o) {
        echo call_name($o), ' ', call_who($o), "\n";
    }
    $l = new Left;
    echo $l->viaParent(), ' ', $l->callPriv(), ' ', get_class(Left::make()), "\n";
    // __call and __callStatic, and the site that sees a real method too.
    $m = new Magic;
    foreach (['real', 'fake'] as $name) {
        echo $m->$name(1, 2), "\n";
    }
    echo $m->missing($r), ' ', Magic::nothing($r), "\n";
    foreach ([new Magic, new Left] as $o) {
        echo $o->name(), "\n";
    }
    // Native methods, and a site that sees a native and a user method.
    $ao = new ArrayObject([3, 1, 2]);
    $it = new ArrayIterator(['a' => 1]);
    echo $ao->count(), ' ', $it->key(), ' ', $it->current(), "\n";
    foreach ([new ArrayObject([1]), new ArrayIterator([1, 2]), new ArrayObject([1, 2, 3])] as $o) {
        echo get_class($o), ' ', $o->count(), "\n";
    }
    // A closure object's methods.
    $f = function ($x) { return $x * 2; };
    $g = function ($x) { return $x + $this->k; };
    $bound = new class { public $k = 100; };
    echo $f->__invoke(3), ' ', $g->call($bound, 4), ' ', ($g->bindTo($bound))(5), "\n";
    // The errors (a private or protected method of the object's own class:
    // #432 for an inherited one).
    $nul = null;
    try { $nul->name(); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    $b = new Base;
    try { $b->priv(); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { $b->prot(); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { $l->nope(); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    $int = 5;
    try { $int->name(); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}

echo "-- sends by reference\n";
function bump(&$x) { $x = ($x ?? 0) + 1; return $x; }
class Sender {
    public $p = 1;
    public $list = [];
    function bump(&$x) { $x = ($x ?? 0) + 10; return $x; }
    static function sbump(&$x) { $x = ($x ?? 0) + 100; return $x; }
    function val($x) { return $x; }
}
for ($r = 0; $r < 2; $r++) {
    unset($u1, $u2, $u3);
    $a = ['k' => 1];
    $s = new Sender;
    // Undefined variables: made null, no warning.
    echo bump($u1), ' ', $s->bump($u2), ' ', Sender::sbump($u3), "\n";
    var_dump($u1, $u2, $u3);
    // Elements (existing and new) and properties (declared and dynamic).
    echo bump($a['k']), ' ', bump($a['new']), ' ', $s->bump($a['k']), ' ', Sender::sbump($a['deep']['er']), "\n";
    echo bump($s->p), ' ', $s->bump($s->p), ' ', Sender::sbump($s->p), "\n";
    $std = new stdClass;
    echo bump($std->dyn), ' ', $s->bump($std->dyn), "\n";
    var_dump($a, $s->p, $std);
    // A variable bound by reference already, and one sent twice.
    $x = 1; $y = &$x;
    echo bump($x), ' ', bump($y), ' ', $x, "\n";
    // (A call's result sent by reference: #429.)
    // Builtins taking a reference.
    $list = [3, 1, 2];
    sort($list);
    preg_match('/(b)/', 'abc', $matches);
    array_push($s->list, $r);
    echo implode(',', $list), ' ', $matches[1], ' ', count($s->list), "\n";
    // By value to a method: the variable is untouched.
    echo $s->val($a), "\n";
}

echo "-- parameter defaults\n";
const LIMIT = 3;
enum Suit: string { case Hearts = 'H'; case Spades = 'S'; }
class Defaults {
    const K = 'k';
    public function __construct(public $v = 'v') {}
    static function make($a = self::K, $b = LIMIT * 2, $c = new Defaults('made'), $d = Suit::Hearts, $e = [LIMIT, self::K], $f = null) {
        return implode(' ', [$a, $b, $c->v, $d->value, implode(',', $e), var_export($f, true)]);
    }
}
function needs_const($x = UNDEFINED_CONST) { return $x; }
for ($r = 0; $r < 2; $r++) {
    echo Defaults::make(), "\n";
    echo Defaults::make('A'), "\n";
    echo Defaults::make('A', 1, new Defaults, Suit::Spades, [], 0), "\n";
    echo Defaults::make(c: new Defaults('named'), f: 'F'), "\n";
    echo Defaults::make(null, null, new Defaults(null), Suit::Hearts, [null]), "\n";
    echo needs_const(5), "\n";
    try { echo needs_const(), "\n"; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}

echo "-- static property writes\n";
class Stat {
    public static $plain = 0;
    public static int $count = 0;
    public static ?Noisy $held = null;
    private static $secret = 's';
    protected static $prot = 'p';
    public static $shared = 'base';
    static function bump() { static::$count = static::$count + 1; self::$secret = 'set ' . static::$count; return self::$secret; }
    static function setProt($v) { static::$prot = $v; return static::$prot; }
}
class StatChild extends Stat {
    public static $shared = 'child';
}
class StatOther extends Stat {}
for ($r = 0; $r < 2; $r++) {
    Stat::$plain = $r;
    Stat::$plain = [1, 2];
    Stat::$plain = 'str';
    $v = Stat::$plain = 'chained';
    echo $v, ' ', Stat::$plain, "\n";
    // Typed: a value already of the type.
    Stat::$count = 10;
    echo Stat::bump(), ' ', StatChild::bump(), ' ', StatOther::bump(), ' ', Stat::$count, "\n";
    // Inherited (one cell) and redeclared (its own).
    StatOther::$plain = 'via other';
    StatChild::$shared = 'child set';
    echo Stat::$plain, ' ', Stat::$shared, ' ', StatChild::$shared, "\n";
    echo Stat::setProt('a'), ' ', StatChild::setProt('b'), "\n";
    // Through a reference.
    $ref = &Stat::$plain;
    Stat::$plain = 'through';
    echo $ref, "\n";
    $ref = 'back';
    echo Stat::$plain, "\n";
    unset($ref);
    // The old value's destructor runs at the write.
    Stat::$held = new Noisy("first $r");
    Stat::$held = new Noisy("second $r");
    echo "after the write\n";
    Stat::$held = null;
    echo "after null\n";
    // The errors.
    try { Stat::$secret = 1; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { Stat::$prot = 1; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { Stat::$undeclared = 1; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { NoSuchClass::$x = 1; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    // An undefined value: the warning, null stored.
    Stat::$plain = $undefined_var;
    var_dump(Stat::$plain);
}

echo "-- static calls asked by reference\n";
class StaticRef {
    static function byRef(&$x) { $x .= '!'; return $x; }
    static function byVal($x) { return $x; }
    static function go(array &$a) { return static::byRef($a['k']) . ' ' . static::byVal($a['k']) . ' ' . self::byRef($a['j']); }
}
class StaticRefChild extends StaticRef {
    static function byRef(&$x) { $x .= '?'; return $x; }
}
for ($r = 0; $r < 2; $r++) {
    $a = ['k' => 'a', 'j' => 'b'];
    $o = new Sender;
    echo StaticRef::byRef($a['k']), ' ', StaticRef::byVal($a['j']), ' ', StaticRef::byRef($o->p), ' ', StaticRef::byRef($a['new']), "\n";
    echo StaticRef::go($a), ' ', StaticRefChild::go($a), "\n";
    var_dump($a, $o->p);
    try { StaticRef::nope($a['k']); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}

echo "-- references to properties\n";
class Props {
    public $plain = 1;
    public int $typed = 2;
    public ?array $list = [1, 2];
    public readonly int $ro;
    public private(set) int $narrow = 3;
    public $hooked { get => 'hooked'; set => null; }
    function __construct() { $this->ro = 4; }
    function inner() { $r = &$this->narrow; $r++; return $this->narrow; }
}
for ($r = 0; $r < 2; $r++) {
    $p = new Props;
    $ref = &$p->plain;
    $ref = 'changed';
    echo $p->plain, "\n";
    unset($ref);
    $t = &$p->typed;
    $t = 20;
    echo $p->typed, "\n";
    unset($t);
    foreach ($p->list as &$v) { $v *= 10; }
    unset($v);
    echo implode(',', $p->list), "\n";
    echo $p->inner(), "\n";
    $std = new stdClass;
    $d = &$std->made;
    $d = 'made';
    var_dump($std);
    unset($d);
    $dyn = &$p->dynamic;
    $dyn = 'dyn';
    echo $p->dynamic, "\n";
    unset($dyn);
    try { $x = &$p->ro; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { $x = &$p->narrow; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    try { $x = &$p->hooked; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    $nul = null;
    try { $x = &$nul->p; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    // (A reference to a property only __get has: #432.)
    unset($x);
}
echo "done\n";
