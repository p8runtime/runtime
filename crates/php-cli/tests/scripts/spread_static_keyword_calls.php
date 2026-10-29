<?php
// parent::m(...$a), self::, static:: with spread arguments: $this and the called class go along.
class Base { public $got = []; function __construct(...$e) { $this->got = $e; echo static::class, " base(", count($e), ")\n"; } static function make(...$a) { return static::class . ':' . implode(',', $a); } function inst(...$a) { return get_class($this) . '>' . implode(',', $a); } }
class Child extends Base {
    function __construct(int ...$e) { parent::__construct(...$e); }
    static function mk(...$a) { return [parent::make(...$a), self::make(...$a), static::make(...$a), Base::make(...$a)]; }
    function call(...$a) { return [parent::inst(...$a), self::inst(...$a), static::inst(...$a)]; }
}
class Grand extends Child {}
$c = new Child(1, 2, 3); var_dump($c->got);
var_dump(Grand::mk(4, 5), (new Grand(6))->call(7, 8));
trait T { function t(...$a) { return parent::inst(...$a) . '|' . self::class; } }
class U extends Base { use T; }
var_dump((new U)->t(9));
