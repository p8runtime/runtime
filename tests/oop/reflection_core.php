<?php
interface I { const IC = 'i'; function f(); }
abstract class Base implements I { const BC = 1; public static $count = 3; protected $p = [1]; private $hidden; public function f() {} abstract protected function g(int $a, $b = 2, ...$rest); }
final class Child extends Base { public int $typed; public readonly string $ro; public function __construct(public $promo = 5) {} protected function g(int $a, $b = 2, ...$rest) { return $a + $b; } public static function s($x) { return "s$x"; } private function priv() { return 'p'; } }
$rc = new ReflectionClass('Child');
var_dump($rc->getName(), $rc->getShortName(), $rc->isFinal(), $rc->isAbstract(), $rc->isInterface(), $rc->isInstantiable(), $rc->getParentClass()->getName());
var_dump($rc->isSubclassOf('Base'), $rc->isSubclassOf('I'), $rc->implementsInterface('I'), $rc->getInterfaceNames(), $rc->hasMethod('G'), $rc->hasProperty('p'), $rc->hasConstant('IC'));
foreach ($rc->getMethods() as $m) echo $m->class ?? '', $m->getName(), ' ', $m->getDeclaringClass()->getName(), ' ', implode(' ', Reflection::getModifierNames($m->getModifiers())), "\n";
foreach ($rc->getProperties() as $p) echo $p->name, ' ', $p->class, ' ', implode(' ', Reflection::getModifierNames($p->getModifiers())), "\n";
$o = new Child(7);
$rp = new ReflectionProperty('Base', 'p'); var_dump($rp->getValue($o)); $rp->setValue($o, [2]); var_dump($rp->getValue($o), $rp->isProtected(), $rp->getDefaultValue());
var_dump((new ReflectionProperty('Child', 'typed'))->isInitialized($o), $rc->getStaticPropertyValue('count'));
$rc->setStaticPropertyValue('count', 9); var_dump(Base::$count);
$g = $rc->getMethod('g'); var_dump($g->invoke($o, 1), $g->invokeArgs($o, [1, 10]), $g->getNumberOfParameters(), $g->getNumberOfRequiredParameters());
foreach ($g->getParameters() as $prm) var_dump($prm->getName(), $prm->getPosition(), $prm->isOptional(), $prm->isDefaultValueAvailable() ? $prm->getDefaultValue() : 'none');
var_dump($rc->getMethod('s')->invoke(null, 'x'), $rc->newInstanceArgs([3])->promo, $rc->newInstance()->promo, $rc->getConstants(), $rc->getConstant('BC'));
try { $rc->getMethod('nope'); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
try { new ReflectionClass('Nope'); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
$ro = new ReflectionObject($o); var_dump($ro->getName(), $ro instanceof ReflectionClass);
echo $rc->getMethod('f')->name, ' ', $rc->getMethod('f')->class, "\n";
function plain(int $a, ?string $b = null) { return "$a$b"; }
$rf = new ReflectionFunction('plain'); var_dump($rf->invoke(1, 'x'), $rf->getNumberOfRequiredParameters(), $rf->getParameters()[1]->allowsNull());
