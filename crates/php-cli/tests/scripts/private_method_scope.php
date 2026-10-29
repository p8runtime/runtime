<?php
// $this->m() in class C's code calls C's private m on a C instance, even
// when the object's class (a subclass) declares its own m (PHPUnit 9's
// TestCase calls its private checkRequirements(); WordPress's test case
// declares a protected one).
class Base {
    function run() { $this->check(); $this->pub(); call_user_func([$this, 'check']); static::name(); }
    private function check() { echo "Base::check\n"; }
    private function pub() { echo "Base::pub\n"; }
    private static function name() { echo "Base::name\n"; }
}
class Child extends Base {
    protected function check() { echo "Child::check\n"; }
    public function pub() { echo "Child::pub\n"; }
    public static function name() { echo "Child::name\n"; }
    function mine() { $this->check(); }
}
(new Child)->run();
(new Child)->mine();
(new Child)->pub();
try { (new Child)->check(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
