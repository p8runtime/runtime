<?php
// `$this->m()` and `$this->a[...] = v` in an instance method take the
// frame's $this: calls (by value and by reference, cached and not, private
// and inherited, __call, first-class callables, pipes), and writes into
// array properties (append, nested keys, compound ops, ArrayAccess
// objects, hooks, __get, readonly).

class Calc {
    public array $log = [];
    public array $grid = [];
    public $store;
    private int $n = 0;
    public function __construct() { $this->store = new ArrayObject(); }
    private function add(int $x): int { $this->n += $x; return $this->n; }
    protected function twice(&$v) { $v *= 2; }
    public function run(): array {
        $out = [];
        for ($i = 1; $i <= 3; $i++) {
            $out[] = $this->add($i);
            $this->log[] = "add $i";
        }
        $v = 5;
        $this->twice($v);
        $this->grid['a']['b'] = 1;
        $this->grid['a']['b'] += 10;
        $this->grid['a']['c'][] = 'x';
        $this->grid['s'] = 'ab';
        $this->grid['s'] .= 'cd';
        $this->store['k'] = 'v';
        $this->store[] = 'pushed';
        $f = $this->add(...);
        $out[] = $f(100);
        $out[] = 7 |> $this->add(...);
        return [$out, $v, $this->grid, $this->store->getArrayCopy(), $this->log];
    }
    public function __call($name, $args) { return "__call $name(" . implode(',', $args) . ")"; }
    public function viaCall() { return $this->nothere(1, 2); }
}
var_dump((new Calc)->run());
var_dump((new Calc)->viaCall());

class ChildCalc extends Calc {
    public function more() { $this->log[] = 'child'; return count($this->run()[4]); }
}
var_dump((new ChildCalc)->more());

// A by-reference parameter through a pipe is refused.
class Pipes {
    public function byRef(array &$a) { $a[] = 1; return $a; }
    public function go() { try { return [1] |> $this->byRef(...); } catch (Error $e) { return $e->getMessage(); } }
}
var_dump((new Pipes)->go());

class Hook {
    public array $list = [] { get => $this->list; set => $this->list = $value; }
    public function push() { try { $this->list[] = 1; } catch (Error $e) { return $e->getMessage(); } }
}
var_dump((new Hook)->push());

class Magic {
    private array $d = [];
    public function __get($n) { echo "get $n\n"; return $this->d[$n] ?? null; }
    public function __set($n, $v) { echo "set $n\n"; $this->d[$n] = $v; }
    public function put() { $this->arr = [1]; return $this->d; }
}
var_dump((new Magic)->put());

class Ro {
    public function __construct(public readonly array $list = []) {}
    public function push() { try { $this->list[] = 1; } catch (Error $e) { return $e->getMessage(); } }
}
var_dump((new Ro)->push());

// No $this.
class NoThis {
    public array $a = [];
    public function m() { return 1; }
    public function callIt() { return $this->m(); }
    public function write() { $this->a[] = 1; return $this->a; }
}
try { NoThis::callIt(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { NoThis::write(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$c = Closure::bind(function () { return $this->callIt() + count($this->write()); }, new NoThis, NoThis::class);
var_dump($c());
