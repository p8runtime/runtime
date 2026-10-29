<?php
// `$a[]` passed where the callee is found as the call runs (a method, a
// function not declared yet): a new element when the parameter takes a
// reference, PHP's "Cannot use [] for reading" Error when it takes a value.
class Z {
    function add(&$slot, $v) { $slot = $v; }
    function value($x) { return $x; }
    function run() {
        $list = [];
        $this->add($list[], 'a');
        $this->add($list[], 'b');
        $this->add($list['k'][], 'c');
        var_dump($list);
        try { $this->value($list[]); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
        later($list[], 'd');
        var_dump(count($list));
    }
}
(new Z)->run();
function later(&$slot, $v) { $slot = $v; }
