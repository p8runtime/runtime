<?php
namespace Lib;

use ReflectionClass;
use ReflectionEnum;
use ReflectionFunction;
use ReflectionMethod;

trait Greets {
    /** trait property */
    public int $greeting = 1;

    /** trait method */
    public function greet() {}
}

trait Logs {}

interface HasId {
    /** interface constant */
    const ID = 1;
}

/**
 * The Widget.
 *
 * @package lib
 */
final class Widget implements HasId {
    use Greets, Logs;

    /** the name */
    const NAME = 'widget';

    /**
     * @var string
     */
    public $label;

    /* an ordinary comment */
    public $plain;

    /** first */
    /** second */
    public $twice;

    /** a, b share it */
    public $a, $b;

    public $undocumented;

    /**
     * Run it.
     */
    public function run(): void
    {
        $x = 1;
    }

    /** one line */ public function oneLine() {}

    public function bare()
    {
    }

    public static function stat() { return 1; }
}

/** An interface. */
interface Shape {
    /** abstract */
    public function area(): float;
}

/**
 * Suit.
 */
enum Suit: string {
    /** hearts doc */
    case Hearts = 'H';
    case Spades = 'S';

    /** enum const */
    const Wild = self::Spades;
}

/** a function */
function documented(
    int $a,
    int $b = 2,
) {
    return $a + $b;
}

function undocumented() {}

function show($v) {
    var_dump($v === false ? false : (is_string($v) ? $v : $v));
}

$rc = new ReflectionClass(Widget::class);
var_dump($rc->getFileName() === __FILE__, $rc->getStartLine(), $rc->getEndLine());
var_dump($rc->getDocComment());
var_dump($rc->getTraitNames());
var_dump(array_keys($rc->getTraits()));
var_dump((new ReflectionClass(HasId::class))->getTraitNames());
var_dump($rc->getReflectionConstant('NAME')->getDocComment());
var_dump($rc->getReflectionConstant('ID')->getDocComment());
foreach (['label', 'plain', 'twice', 'a', 'b', 'undocumented', 'greeting'] as $p) {
    echo $p, ': ';
    var_dump($rc->getProperty($p)->getDocComment());
}
foreach (['run', 'oneLine', 'bare', 'stat', 'greet'] as $m) {
    $rm = $rc->getMethod($m);
    echo $m, ': ', $rm->getStartLine(), '-', $rm->getEndLine(), ' ';
    var_dump($rm->getDocComment());
}
var_dump($rc->getMethod('greet')->getFileName() === __FILE__);

$i = new ReflectionClass(Shape::class);
var_dump($i->getDocComment(), $i->getStartLine(), $i->getEndLine());
var_dump($i->getMethod('area')->getDocComment());

$e = new ReflectionEnum(Suit::class);
var_dump($e->getDocComment(), $e->getStartLine(), $e->getEndLine());
var_dump($e->isBacked(), (string) $e->getBackingType());
var_dump($e->hasCase('Hearts'), $e->hasCase('Wild'), $e->hasCase('Nope'));
foreach ($e->getCases() as $case) {
    echo get_class($case), ' ', $case->getName(), ' ', $case->getBackingValue(), ' ';
    var_dump($case->getDocComment());
}
var_dump($e->getCase('Spades')->getValue());
var_dump($e->getCase('Hearts')->getEnum()->getName());
var_dump($e->getReflectionConstant('Wild')->getDocComment());
try {
    $e->getCase('Wild');
} catch (\ReflectionException $ex) {
    echo $ex->getMessage(), "\n";
}
try {
    $e->getCase('Nope');
} catch (\ReflectionException $ex) {
    echo $ex->getMessage(), "\n";
}
try {
    new ReflectionEnum(Widget::class);
} catch (\ReflectionException $ex) {
    echo $ex->getMessage(), "\n";
}

$f = new ReflectionFunction('Lib\documented');
var_dump($f->getDocComment(), $f->getStartLine(), $f->getEndLine(), $f->getFileName() === __FILE__);
$g = new ReflectionFunction('Lib\undocumented');
var_dump($g->getDocComment(), $g->getStartLine(), $g->getEndLine());
$c = function () {
    return 1;
};
$rf = new ReflectionFunction($c);
var_dump($rf->getStartLine(), $rf->getEndLine());

// internal ones have none
$s = new ReflectionClass('ArrayObject');
var_dump($s->getFileName(), $s->getStartLine(), $s->getEndLine(), $s->getDocComment());
$s = new ReflectionFunction('strlen');
var_dump($s->getFileName(), $s->getStartLine(), $s->getEndLine(), $s->getDocComment());
var_dump((new ReflectionMethod('ArrayObject', 'count'))->getEndLine());
