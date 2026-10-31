<?php
// PHP 8.4 asymmetric visibility: who may write a property.

class Bar { public int $n = 0; }

class Foo {
    public private(set) int $count = 0;
    public protected(set) string $name = 'foo';
    private(set) array $items = [];
    public private(set) ?Bar $bar = null;
    public private(set) int $late;

    public function __construct(
        public private(set) string $id = 'id',
        protected(set) int $promoted = 1,
        private(set) string $bare = 'bare',
    ) {
        $this->bar = new Bar;
    }

    public function bump() { $this->count++; $this->count += 2; return $this->count; }
    public function add($x) { $this->items[] = $x; return count($this->items); }
    public function rename($n) { $this->name = $n; }
    public function refBump() { $r = &$this->count; $r++; return $this->count; }
    public function clear() { unset($this->count); }
    public function initLate() { $this->late = 7; }
}

class Child extends Foo {
    public function renameChild($n) { $this->name = $n; return $this->name; }
    public function bumpChild() { $this->count = 99; }
    public function promotedChild() { $this->promoted = 5; return $this->promoted; }
}

class Stranger {
    public function poke(Foo $f) { $f->name = 'x'; }
    public static function pokeStatic(Foo $f) { $f->count = 1; }
}

function attempt(string $what, callable $fn) {
    try {
        $fn();
        echo "$what: ok\n";
    } catch (Error $e) {
        echo "$what: ", get_class($e), ": ", $e->getMessage(), "\n";
    }
}

$f = new Foo;
echo "read: {$f->count} {$f->name} {$f->id} {$f->promoted} {$f->bare}\n";

// From global scope (attempt's closures are global code).
attempt('assign', function () use ($f) { $f->count = 5; });
attempt('compound', function () use ($f) { $f->count += 1; });
attempt('concat', function () use ($f) { $f->name .= '!'; });
attempt('pre-inc', function () use ($f) { ++$f->count; });
attempt('post-dec', function () use ($f) { $f->count--; });
attempt('append', function () use ($f) { $f->items[] = 1; });
attempt('dim set', function () use ($f) { $f->items['k'] = 1; });
attempt('unset', function () use ($f) { unset($f->count); });
attempt('unset dim', function () use ($f) { unset($f->items[0]); });
attempt('reference', function () use ($f) { $r = &$f->count; });
attempt('assign ref', function () use ($f) { $x = 1; $f->count = &$x; });
attempt('by-ref arg', function () use ($f) { sort($f->items); });
attempt('foreach by ref', function () use ($f) { foreach ($f->items as &$v) {} });
attempt('promoted', function () use ($f) { $f->id = 'new'; });
attempt('promoted protected', function () use ($f) { $f->promoted = 2; });
attempt('promoted bare', function () use ($f) { $f->bare = 'z'; });
attempt('uninitialized', function () use ($f) { $f->late = 1; });
attempt('null coalesce assign (set)', function () use ($f) { $f->count ??= 3; });
attempt('list assign', function () use ($f) { [$f->count] = [9]; });

// A held object is not the property: writing into it is fine.
attempt('nested write', function () use ($f) { $f->bar->n = 42; });
echo "bar->n: {$f->bar->n}\n";
attempt('object reference', function () use ($f) { $r = &$f->bar; $r = null; });
var_dump($f->bar instanceof Bar);
attempt('assign ref to object prop', function () use ($f) { $x = new Bar; $f->bar = &$x; });

// From inside the class, a subclass, and an unrelated class.
echo "bump: ", $f->bump(), "\n";
echo "add: ", $f->add('a'), "\n";
echo "refBump: ", $f->refBump(), "\n";
$f->initLate();
echo "late: {$f->late}\n";
$c = new Child;
echo "renameChild: ", $c->renameChild('child'), "\n";
attempt('private(set) from subclass', function () use ($c) { $c->bumpChild(); });
echo "promotedChild: ", $c->promotedChild(), "\n";
attempt('protected(set) from unrelated class', function () use ($f) { (new Stranger)->poke($f); });
attempt('private(set) from static method', function () use ($f) { Stranger::pokeStatic($f); });
$f->clear();
attempt('read after unset', function () use ($f) { echo $f->count; });

// Closures bound to a scope take its rights.
$set = function ($v) { $this->count = $v; return $this->count; };
$h = new Foo;
echo "bound: ", Closure::bind($set, $h, Foo::class)(100), "\n";
attempt('bound to unrelated', function () use ($set, $h) { Closure::bind($set, $h, Stranger::class)(1); });
attempt('bound to subclass', function () use ($set, $h) { Closure::bind($set, $h, Child::class)(1); });
