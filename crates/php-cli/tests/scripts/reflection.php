<?php
namespace App;
interface Shape { function area(): float; }
abstract class Base implements Shape { const KIND = 'base'; protected static $count = 0; public $tag = 't'; }
final class Circle extends Base {
    const PI = 3.14;
    private float $r;
    public ?string $label = null;
    public function __construct(float $r = 1.0, public int $id = 0) { $this->r = $r; }
    public function area(): float { return self::PI * $this->r ** 2; }
    private function secret(int $x, string ...$rest): string { return "s$x"; }
    public static function make(int|string $n = 2): static { return new static((float) $n); }
}
$rc = new \ReflectionClass(Circle::class);
var_dump($rc);
var_dump($rc->getName(), $rc->getShortName(), $rc->getNamespaceName(), $rc->inNamespace());
var_dump($rc->isFinal(), $rc->isAbstract(), $rc->isInterface(), $rc->isInstantiable(), $rc->isUserDefined(), $rc->isInternal());
var_dump($rc->getParentClass()->getName(), $rc->implementsInterface(Shape::class), $rc->isSubclassOf(Base::class));
var_dump($rc->getInterfaceNames());
var_dump($rc->hasMethod('area'), $rc->hasMethod('nope'), $rc->hasProperty('r'), $rc->hasConstant('PI'));
foreach ($rc->getMethods() as $m) echo $m->class, '::', $m->getName(), ' ', implode(' ', \Reflection::getModifierNames($m->getModifiers()) ?: []), "\n";
$c = $rc->newInstance(2.0, 7);
var_dump($c->area(), $c->id);
$c2 = $rc->newInstanceArgs([3.0]);
var_dump($c2->area());
$raw = $rc->newInstanceWithoutConstructor(); var_dump($raw->id ?? 'unset');
$m = $rc->getMethod('secret');
var_dump($m->isPrivate(), $m->isStatic(), $m->getNumberOfParameters(), $m->getNumberOfRequiredParameters(), $m->isVariadic());
var_dump($m->invoke($c, 5, 'a'));
foreach ($m->getParameters() as $p) { echo $p->getPosition(), ' ', $p->getName(), ' ', $p->getType(), ' ', var_export($p->isOptional(), true), ' ', var_export($p->isVariadic(), true), "\n"; }
$mk = new \ReflectionMethod(Circle::class, 'make');
var_dump($mk->isStatic(), (string) $mk->getReturnType());
$t = $mk->getParameters()[0]->getType(); var_dump(get_class($t), (string) $t);
foreach ($t->getTypes() as $tt) echo $tt->getName(), ' ', var_export($tt->isBuiltin(), true), "\n";
var_dump($mk->getParameters()[0]->getDefaultValue(), $mk->invoke(null, 4)->area());
$ctor = $rc->getConstructor();
foreach ($ctor->getParameters() as $p) { var_dump($p->getName(), $p->isDefaultValueAvailable() ? $p->getDefaultValue() : null, $p->isPromoted(), $p->allowsNull()); }
$pr = new \ReflectionProperty(Circle::class, 'r');
var_dump($pr->isPrivate(), $pr->getValue($c), (string) $pr->getType(), $pr->hasDefaultValue());
$pr->setValue($c, 10.0); var_dump($c->area());
$lp = $rc->getProperty('label'); var_dump($lp->getType()->allowsNull(), $lp->getType()->getName(), $lp->getDefaultValue(), $lp->isInitialized($c));
foreach ($rc->getProperties() as $p) echo $p->class, '::$', $p->getName(), "\n";
var_dump($rc->getConstants(), $rc->getConstant('KIND'), $rc->getStaticPropertyValue('count'));
$rf = new \ReflectionFunction(function (int $a, $b = [1]) use ($c): ?int { return $a; });
var_dump($rf->getNumberOfParameters(), $rf->isClosure(), (string) $rf->getReturnType(), $rf->invoke(9), $rf->getParameters()[1]->getDefaultValue());
$sl = new \ReflectionFunction('strlen'); var_dump($sl->getName(), $sl->isInternal(), $sl->getNumberOfParameters(), $sl->getParameters()[0]->getName(), (string) $sl->getReturnType(), $sl->invoke('abcd'));
try { new \ReflectionClass('Nope\Missing'); } catch (\ReflectionException $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { $rc->getMethod('zzz'); } catch (\ReflectionException $e) { echo $e->getMessage(), "\n"; }
try { new \ReflectionProperty(Circle::class, 'zz'); } catch (\ReflectionException $e) { echo $e->getMessage(), "\n"; }
$ro = new \ReflectionObject($c); var_dump($ro->getName(), $ro instanceof \ReflectionClass);
var_dump((new \ReflectionClass('ArrayAccess'))->isInterface(), (new \ReflectionClass('Exception'))->isInternal());
