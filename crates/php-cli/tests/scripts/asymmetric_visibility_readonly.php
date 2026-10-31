<?php
// readonly properties are implicitly protected(set) (PHP 8.4), and the
// set visibility under inheritance and reflection.

class P {
    public readonly int $pDefault;
    public private(set) readonly int $pPrivate;
    public protected(set) readonly int $pProtected;
    public public(set) readonly int $pPublic;
    protected readonly int $prot;

    public function __construct() {
        $this->pDefault = 1;
        $this->pPrivate = 1;
        $this->pProtected = 1;
        $this->pPublic = 1;
    }
}

class C extends P {
    public function __construct() {
        $this->pDefault = 2;
        try {
            $this->pPrivate = 2;
        } catch (Error $e) {
            echo $e->getMessage(), "\n";
        }
        $this->pProtected = 2;
        $this->pPublic = 2;
        $this->prot = 2;
        echo "C initialized: {$this->pDefault} {$this->pProtected} {$this->pPublic} {$this->prot}\n";
    }
}

new P;
new C;
$p = (new ReflectionClass(P::class))->newInstanceWithoutConstructor();
foreach (['pDefault', 'pPrivate', 'pProtected', 'pPublic'] as $name) {
    try {
        $p->$name = 3;
        echo "$name = {$p->$name}\n";
    } catch (Error $e) {
        echo $e->getMessage(), "\n";
    }
}
try {
    $p->pPublic = 4;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
$c = new C;
try {
    $c->pDefault = 5;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}

// protected(set) is checked against the class that first declared the
// property, so a sibling class may write it.
class Base { public protected(set) int $v = 0; }
class Left extends Base {}
class Right extends Base {
    public function poke(Left $l) { $l->v = 9; return $l->v; }
}
echo "sibling: ", (new Right)->poke(new Left), "\n";

// Redeclaring: a subclass may widen the set visibility, or drop it.
class A1 { public protected(set) string $foo = 'a'; }
class B1 extends A1 { public string $foo = 'b'; }
$b = new B1;
$b->foo = 'widened';
echo $b->foo, "\n";
class A2 { public protected(set) string $foo = 'a'; }
class B2 extends A2 { public public(set) string $foo = 'b'; }
$b = new B2;
$b->foo = 'public(set)';
echo $b->foo, "\n";
class A3 { public protected(set) string $foo = 'a'; }
class B3 extends A3 { public protected(set) string $foo = 'b'; public function set($v) { $this->foo = $v; } }
$b = new B3;
$b->set('redeclared');
echo $b->foo, "\n";
try {
    $b->foo = 'no';
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}

// A hooked property's set hook runs only where it may be written.
class Hooked {
    public private(set) int $h = 0 {
        set { $this->h = $value * 2; }
    }
    public function setH($v) { $this->h = $v; return $this->h; }
}
$hk = new Hooked;
echo "hook: ", $hk->setH(4), "\n";
try {
    $hk->h = 1;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}

// Reflection.
class R {
    public private(set) int $a = 0;
    public protected(set) int $b = 0;
    protected private(set) int $c = 0;
    public readonly int $d;
    public public(set) readonly int $e;
    protected protected(set) int $f = 0;
    private private(set) int $g = 0;
    public int $h = 0;
    public function __construct(private(set) int $i = 0) {}
}
$rc = new ReflectionClass(R::class);
foreach ($rc->getProperties() as $prop) {
    printf(
        "%s: modifiers=%d privateSet=%s protectedSet=%s final=%s %s\n",
        $prop->getName(),
        $prop->getModifiers(),
        var_export($prop->isPrivateSet(), true),
        var_export($prop->isProtectedSet(), true),
        var_export($prop->isFinal(), true),
        implode(' ', Reflection::getModifierNames($prop->getModifiers())),
    );
}
echo "IS_PRIVATE_SET=", ReflectionProperty::IS_PRIVATE_SET, " IS_PROTECTED_SET=", ReflectionProperty::IS_PROTECTED_SET, "\n";
echo "private(set): ", implode(',', array_map(fn($p) => $p->getName(), $rc->getProperties(ReflectionProperty::IS_PRIVATE_SET))), "\n";
echo "protected(set): ", implode(',', array_map(fn($p) => $p->getName(), $rc->getProperties(ReflectionProperty::IS_PROTECTED_SET))), "\n";
// Reflection writes regardless of the set visibility.
$r = new R;
(new ReflectionProperty(R::class, 'a'))->setValue($r, 42);
echo "setValue: {$r->a}\n";
