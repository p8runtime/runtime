<?php
class T {
    private function hidden() { return 'hidden'; }
    public function __call($n, $a) { echo "__call $n(", implode(',', $a), ")\n"; return strtoupper($n); }
    public static function __callStatic($n, $a) { echo "__callStatic $n(", implode(',', $a), ")\n"; return $n; }
    public function inside() { return [self::viaSelf(1), static::viaStatic(2), T::viaName(3)]; }
}
class U extends T {
    public function viaParent() { return parent::missing(4); }
}
$t = new T;
var_dump($t->foo(1, 2), $t->hidden(), T::bar(5), $t->inside());
var_dump(call_user_func([$t, 'cb'], 'x'), call_user_func(['T', 'scb'], 'y'), call_user_func('T::scb2'));
var_dump(array_map([$t, 'mapped'], [1, 2]), is_callable([$t, 'anything']), is_callable(['T', 'anything']));
var_dump((new U)->viaParent());
$f = $t->firstClass(...); var_dump($f(9));
class N { }
try { (new N)->nope(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { N::nope(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
class OnlyCall { function __call($n, $a) { return "call:$n"; } }
try { var_dump(OnlyCall::stat()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
class Gen { function __call($n, $a) { yield $n; yield count($a); } }
foreach ((new Gen)->g(1, 2, 3) as $v) var_dump($v);
