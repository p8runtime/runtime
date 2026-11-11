<?php
// Calls and returns (#413): `static::` in methods the loop calls and
// constructs (their frame takes the called class from `$this`), closures
// made there, arguments past the parameters, a variable scope shared with
// eval, foreach over Iterators through the method cache (another class's
// scope, a class that changes), and traces from callbacks of builtins
// called again and again (their entries are used again).

class Base {
    const NAME = 'base';
    public $made;
    public function __construct() { $this->made = static::class; }
    public static function make() { return new static; }
    public function tag() { return 'tag:' . static::class; }
    public function who() { return static::class . ' ' . static::NAME . ' ' . get_class(new static) . ' ' . (new static)->tag(); }
    public function maker() { return function () { return static::class; }; }
    public function smaker() { return static function () { return static::class; }; }
    public function viaSelf() { return self::class . '/' . static::class; }
    public function isStatic($x) { return $x instanceof static; }
    public function chain() { return static::make()->viaSelf(); }
}
class Child extends Base { const NAME = 'child'; }

$objs = [new Base, new Child];
for ($i = 0; $i < 3; $i++) {
    foreach ($objs as $o) {
        echo $o->who(), ' | ', $o->made, ' | ', ($o->maker())(), ' | ', ($o->smaker())(), ' | ', $o->viaSelf(), ' | ',
            var_export($o->isStatic(new Child), true), ' | ', $o->chain(), "\n";
    }
}
echo get_class(Child::make()), "\n";

function extra($a) { return count(func_get_args()) . ':' . implode(',', func_get_args()); }
function variadic($a, ...$rest) { return $a . '+' . implode(',', $rest); }
function ev() { $x = 1; eval('$x++; $y = 5;'); return $x + $y; }
for ($i = 0; $i < 3; $i++) {
    echo extra(1, 2, $i), ' ', variadic($i, 'a', 'b'), ' ', ev(), ' ', extra($i), "\n";
}

class It implements Iterator {
    private $i = 0;
    function __construct(private array $items) {}
    function current(): mixed { return $this->items[$this->i]; }
    function key(): mixed { return 'k' . $this->i; }
    function next(): void { $this->i++; }
    function rewind(): void { $this->i = 0; }
    function valid(): bool { return $this->i < count($this->items); }
}
class Upper extends It {
    function current(): mixed { return strtoupper(parent::current()); }
}
class Walk {
    function all($it) { $s = ''; foreach ($it as $k => $v) { $s .= "$k=$v,"; } return $s; }
}
for ($i = 0; $i < 2; $i++) {
    foreach ([new It(['a', 'b']), new Upper(['c', 'd']), new It(['e'])] as $it) {
        $s = '';
        foreach ($it as $k => $v) { $s .= "$k=$v,"; }
        echo $s, ' ', (new Walk)->all($it), "\n";
    }
}

function names() { return implode('<', array_map(fn ($f) => ($f['class'] ?? '') . ($f['type'] ?? '') . $f['function'] . '(' . count($f['args'] ?? []) . ')', debug_backtrace())); }
class M { function each($xs) { return array_map(fn ($x) => names() . ':' . $x, $xs); } }
for ($i = 0; $i < 2; $i++) {
    $seen = [];
    $kept = array_filter([1, 0, 2], function ($x) use (&$seen) { $seen[] = names(); return $x > 0; });
    echo implode(' ', (new M)->each([1, 2])), ' | ', implode(',', $kept), ' | ', $seen[0], "\n";
}
