<?php
// Static-syntax calls (`C::m()`, `self::`, `parent::`, `static::`), every
// site run three times or more so the later runs take the site's cache
// (the loop's StaticCall): what a site remembered must not answer a call
// that differs in class, scope, `$this` or arguments.
spl_autoload_register(function ($c) {
    echo "autoload $c\n";
    if ($c === 'Late') {
        eval('class Late { static function hi($x) { return "late $x"; } }');
    }
});

// The called class: kept by self::, parent:: and static::, not by a name.
class Base {
    static function who($x = '') { return static::class . $x; }
    static function label($x) { return "Base#$x"; }
    static function make($x) { return static::label($x); }
    static function viaSelf($x) { return self::who($x); }
}
class Kid extends Base {
    static function label($x) { return "Kid#$x"; }
    static function viaParent($x) { return parent::who($x); }
    static function viaSelf2($x) { return self::who($x); }
    static function viaBase($x) { return Base::who($x); }
    static function viaKid($x) { return Kid::who($x); }
    static function callables() { return [self::who(...), parent::who(...), static::label(...)]; }
}
class Kid2 extends Base { static function label($x) { return "Kid2#$x"; } }
class GrandKid extends Kid { }

// An instance method called with static syntax: `$this` kept when it is of
// the class's kind, an Error when it is not or there is none.
class P {
    public $tag;
    function __construct($tag) { $this->tag = $tag; }
    function describe($x) { return "describe " . static::class . " {$this->tag} $x"; }
    function run($x) { return Q::inst($x); }
    static function runStatic($x) { return Q::inst($x); }
}
class Q extends P {
    function __construct($tag) { parent::__construct("q-$tag"); }
    function inst($x) { return "inst " . static::class . " " . get_class($this) . " {$this->tag} $x"; }
    function up($x) { return parent::describe($x) . " + " . self::inst($x) . " + " . static::inst($x); }
}
class Q2 extends Q { }
class R extends P { }

// __callStatic, and __call when there is a `$this` of the class's kind.
class Magic {
    static function __callStatic($m, $a) { return "callStatic $m(" . implode(',', $a) . ")"; }
    function __call($m, $a) { return "call $m(" . implode(',', $a) . ")"; }
    function viaThis($x) { return self::nothing($x) . " " . Magic::nothing2($x) . " " . static::nothing3($x); }
    static function viaStatic($x) { return self::nothing($x); }
    static function tryMaybe($x) { return static::maybe($x); }
}
class MagicKid extends Magic { static function maybe($x) { return "MagicKid::maybe $x"; } }

// Private and protected methods, from inside, a subclass, and closures
// bound to one scope after another.
class Vault {
    private static function secret($x) { return "secret $x"; }
    protected static function guarded($x) { return "guarded $x " . static::class; }
    static function open($x) { return self::secret($x) . " " . static::guarded($x); }
}
class VaultKid extends Vault {
    static function peek($x) { return parent::guarded($x); }
    static function steal($x) { return Vault::secret($x); }
}

abstract class Shape {
    abstract static function name($x);
    static function show($x) { return static::name($x); }
    static function bad($x) { return self::name($x); }
}
class Circle extends Shape { static function name($x) { return "circle $x"; } }

interface HasFallback { static function fallback(); }
enum Suit: string implements HasFallback {
    case Hearts = 'h';
    case Spades = 's';
    static function fallback(): self { return self::Spades; }
    static function fromChar($c) { return self::tryFrom($c) ?? static::fallback(); }
    static function describe(self $s, $x) { return $s->name . $x; }
    function label($x) { return self::describe($this, $x); }
}

// A hook's `parent::$p::get()`: the parent's hook, or the plain property.
class HookBase {
    public $v = 1 { get => $this->v * 2; set { $this->v = $value + 1; } }
    public $plain = 5;
}
class HookKid extends HookBase {
    public $v { get => parent::$v::get() + 100; set { parent::$v::set($value * 10); } }
    public $plain { get => parent::$plain::get() + 1; }
}

class Args {
    static function byRef(&$a, $b) { $a[] = $b; return count($a); }
    static function two($a, $b = 'dflt') { return var_export($a, true) . "/$b"; }
    static function need($a, $b) { return "$a$b"; }
    static function gen($n) { yield $n; yield $n + 1; }
    static function mixed($a, &$b, $c) { $b = "$a$c"; return $b; }
}

function later($x) { return Later::hi($x); }
function shown(callable $f) {
    try {
        echo $f(), "\n";
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

$secret = function ($x) { return Vault::secret($x); };
$guarded = function ($x) { return Vault::guarded($x); };
$who = function ($x) { return static::who($x) . " " . self::label($x); };
$inst = function ($x) { return Q::inst($x); };
$list = [];
for ($i = 0; $i < 4; $i++) {
    echo "-- $i\n";
    // One static:: site, another class each time.
    echo Base::make($i), " ", Kid::make($i), " ", Kid2::make($i), " ", GrandKid::make($i), " ", Base::make($i), "\n";
    echo Base::viaSelf($i), " ", Kid::viaSelf($i), " ", Kid::viaParent($i), " ", GrandKid::viaParent($i), " ", GrandKid::viaSelf2($i), "\n";
    echo GrandKid::viaBase($i), " ", GrandKid::viaKid($i), " ", Kid::viaKid($i), " ", Base::who(), " ", Kid::who(), "\n";
    foreach ([...Kid::callables(), ...GrandKid::callables(), Kid2::make(...), Base::who(...)] as $f) { echo $f($i), " "; }
    echo "\n";

    foreach ([new Q("a$i"), new Q2("b$i"), new R("c$i"), new P("d$i"), new Q("e$i")] as $o) {
        shown(fn () => $o->run($i));
    }
    shown(fn () => P::runStatic($i));
    shown(fn () => Q::inst($i));
    echo (new Q("u$i"))->up($i), "\n", (new Q2("v$i"))->up($i), "\n";
    foreach ([[new Q("w$i"), Q::class], [null, Q::class], [new R("x$i"), P::class], [new Q2("y$i"), P::class], [new Q2("z$i"), Q2::class]] as [$this_, $scope]) {
        shown(fn () => Closure::bind($inst, $this_, $scope)($i));
    }

    echo Magic::nothing($i, 'a'), " ", (new Magic)->viaThis($i), " ", Magic::viaStatic($i), " ", (new MagicKid)->viaThis($i), "\n";
    echo Magic::tryMaybe($i), " ", MagicKid::tryMaybe($i), " ", Magic::tryMaybe($i), "\n";

    echo Vault::open($i), " | ", VaultKid::open($i), " | ", VaultKid::peek($i), "\n";
    shown(fn () => VaultKid::steal($i));
    shown(fn () => Vault::secret($i));
    shown(fn () => Vault::guarded($i));
    foreach ([Vault::class, VaultKid::class, Vault::class, Base::class, Vault::class] as $scope) {
        shown(fn () => Closure::bind($secret, null, $scope)($i));
        shown(fn () => Closure::bind($guarded, null, $scope)($i));
    }
    foreach ([Base::class, Kid::class, GrandKid::class, Kid2::class, Base::class] as $scope) {
        echo Closure::bind($who, null, $scope)($i), " ";
    }
    echo "\n";

    echo Circle::show($i), " ";
    shown(fn () => Shape::show($i));
    shown(fn () => Circle::bad($i));
    echo Circle::show($i), " ", Circle::name($i), "\n";
    shown(fn () => Shape::name($i));

    echo Suit::fromChar('h')->name, " ", Suit::fromChar('x')->name, " ", Suit::Hearts->label($i), " ", Suit::from('s')->name, " ", Suit::fallback()->value, "\n";
    shown(fn () => HasFallback::fallback());

    $h = new HookKid;
    echo $h->v, " ", $h->plain, " ";
    $h->v = $i;
    echo $h->v, "\n";

    echo Late::hi($i), "\n";
    shown(fn () => later($i));
    if ($i == 1) {
        eval('class Later { static function hi($x) { return "later $x"; } }');
    }

    echo Args::byRef($list, $i), " ", Args::byRef($fresh, $i), " ", count($list), " ", count($fresh), "\n";
    echo Args::two($undef), " ", Args::two($i, $i), " ", Args::two($list), "\n";
    echo Args::mixed($i, $made, 'm'), " $made ", Args::mixed($undef2, $made, 'n'), " $made\n";
    shown(fn () => Args::need($i));
    $one = $i;
    try {
        echo Args::need($one), "\n";
    } catch (ArgumentCountError $e) {
        echo get_class($e), "\n";
    }
    foreach (Args::gen($i) as $v) { echo "g$v "; }
    echo "\n";
    unset($fresh, $made);
}
shown(fn () => Missing::m());
echo "done\n";
