<?php
// The handler of a warning suspends the fiber it runs in: PHP switches
// fibers there and goes on with the statement when the fiber is resumed.
// Around a method call's sends (`$o->two($v, $undefined)`: the method is
// found, `$v` is sent, the warning's handler suspends, the call runs
// after the resume), and around a plain read.
class T {
    function two($a, $b) { return 'two ' . var_export($a, true) . ' ' . var_export($b, true); }
}
set_error_handler(function ($no, $msg) {
    $back = Fiber::suspend("suspended in the handler: $msg");
    echo "resumed with $back\n";
    return true;
});
for ($i = 0; $i < 2; $i++) {
    $fiber = new Fiber(function () {
        $o = new T;
        $v = 'v';
        $r = $o->two($v, $undefined);
        return "$r $v";
    });
    echo $fiber->start(), "\n";
    $fiber->resume("r$i");
    echo $fiber->getReturn(), "\n";
}
$fiber = new Fiber(function () {
    $r = 'read ' . var_export($undefined, true);
    return $r;
});
echo $fiber->start(), "\n";
$fiber->resume('plain');
echo $fiber->getReturn(), "\n";
