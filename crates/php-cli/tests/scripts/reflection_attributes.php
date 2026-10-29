<?php
namespace App;

use Attribute;
use ReflectionAttribute;
use ReflectionClass;
use ReflectionFunction;

#[Attribute(Attribute::TARGET_CLASS | Attribute::TARGET_METHOD | Attribute::IS_REPEATABLE)]
class Tag {
    public function __construct(public string $name = "", public int $weight = 0) {}
}

#[Attribute]
class Plain {}

class NotAttr {}

#[Attribute(Attribute::TARGET_PROPERTY)]
class PropOnly {}

interface Marker {}

#[Attribute]
class Special implements Marker {
    const LEVEL = 3;
}

#[Tag("a", weight: 5), Tag("b")]
#[Plain]
final class Foo {
    #[Tag("const")]
    const X = 1;

    #[PropOnly]
    public $p;

    #[Tag("m", weight: self::X + 1)]
    public function m(#[Tag("param")] $a, $b = 2): void
    {
    }

    #[Special, Tag(name: "named")]
    public static function n() {}
}

#[Tag("f"), Special(), Tag("g")]
function fn1(#[\SensitiveParameter] $x) {
}

function show(array $attrs) {
    foreach ($attrs as $a) {
        echo get_class($a), ' ', $a->getName(), ' target=', $a->getTarget(), ' repeated=', var_export($a->isRepeated(), true), ' args=', json_encode($a->getArguments()), "\n";
    }
}

$rc = new ReflectionClass(Foo::class);
show($rc->getAttributes());
echo "-- name filter (case-insensitive)\n";
show($rc->getAttributes('app\tag'));
show($rc->getAttributes(Plain::class));
show($rc->getAttributes('Nope'));
echo "-- instances\n";
$t = $rc->getAttributes(Tag::class)[0]->newInstance();
var_dump($t->name, $t->weight);
$t = $rc->getAttributes(Tag::class)[1]->newInstance();
var_dump($t->name, $t->weight);

echo "-- methods, properties, constants, parameters, functions\n";
show($rc->getMethod('m')->getAttributes());
show($rc->getMethod('m')->getParameters()[0]->getAttributes());
show($rc->getMethod('m')->getParameters()[1]->getAttributes());
show($rc->getMethod('n')->getAttributes());
show($rc->getProperty('p')->getAttributes());
show($rc->getReflectionConstant('X')->getAttributes());
$f = new ReflectionFunction(__NAMESPACE__ . '\fn1');
show($f->getAttributes());
show($f->getParameters()[0]->getAttributes());
show((new ReflectionFunction(#[Tag("closure")] fn() => 1))->getAttributes());
echo get_class($rc->getProperty('p')->getAttributes()[0]->newInstance()), "\n";

echo "-- IS_INSTANCEOF\n";
show($rc->getMethod('n')->getAttributes(Marker::class, ReflectionAttribute::IS_INSTANCEOF));
show($rc->getMethod('n')->getAttributes(Marker::class));
show((new ReflectionFunction(__NAMESPACE__ . '\fn1'))->getAttributes(Marker::class, ReflectionAttribute::IS_INSTANCEOF));

echo "-- errors\n";
#[NotAttr]
#[Missing]
#[PropOnly]
#[Plain, Plain]
class Bad {}
foreach ((new ReflectionClass(Bad::class))->getAttributes() as $a) {
    try {
        $a->newInstance();
        echo "ok\n";
    } catch (\Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
try {
    $rc->getAttributes(null, 5);
} catch (\Throwable $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
try {
    $rc->getAttributes('Nope', ReflectionAttribute::IS_INSTANCEOF);
} catch (\Throwable $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
echo "-- builtin attribute classes\n";
$a = (new ReflectionClass(Attribute::class))->getAttributes()[0];
echo $a->getName(), ' ', json_encode($a->getArguments()), ' ', $a->newInstance()->flags, "\n";
