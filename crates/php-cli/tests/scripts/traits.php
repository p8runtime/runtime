<?php
trait Hello { public function hello() { return "Hello from " . __CLASS__ . " via " . __TRAIT__; } }
trait World { public $planet = 'earth'; public function world() { return "World " . $this->planet; } public static $count = 0; static function inc() { return ++static::$count; } }
class Greeter { use Hello, World; }
$g = new Greeter;
echo $g->hello(), " / ", $g->world(), "\n";
echo Greeter::inc(), Greeter::inc(), "\n";
class Other { use World; }
echo Other::inc(), "\n";
trait A { public function talk() { return "A"; } public function only() { return "onlyA"; } }
trait B { public function talk() { return "B"; } }
class Talker { use A, B { B::talk insteadof A; A::talk as aTalk; only as protected hidden; } function viaHidden() { return $this->hidden(); } }
$t = new Talker; echo $t->talk(), $t->aTalk(), $t->only(), $t->viaHidden(), "\n";
trait Counter { private int $n = 0; function bump(): static { $this->n++; return $this; } function n() { return $this->n; } }
class C1 { use Counter; } echo (new C1)->bump()->bump()->n(), "\n";
trait AbstractT { abstract function need(): string; function use_it() { return "got " . $this->need(); } }
class Impl { use AbstractT; function need(): string { return "impl"; } } echo (new Impl)->use_it(), "\n";
class Base { function hi() { return "base"; } }
trait Ovr { function hi() { return "trait over " . parent::hi(); } }
class Kid extends Base { use Ovr; } echo (new Kid)->hi(), "\n";
class Own { use Ovr; function hi() { return "own wins"; } } echo (new Own)->hi(), "\n";
trait Nested { use Hello; function both() { return $this->hello() . "!"; } }
class N { use Nested; } echo (new N)->both(), "\n";
trait St { static function make() { return new static; } function cls() { return static::class . "/" . self::class . "/" . get_class($this); } }
class S1 { use St; } class S2 extends S1 {} echo S2::make()->cls(), "\n";
try { new Hello; } catch (Error $e) { echo $e->getMessage(), "\n"; }
