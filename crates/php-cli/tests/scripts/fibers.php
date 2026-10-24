<?php
function inner($x) {
    echo "inner before $x\n";
    $got = Fiber::suspend($x * 2);
    echo "inner got $got\n";
    return $got + 1;
}
$f = new Fiber(function (int $start) {
    echo "fiber start $start\n";
    $a = inner($start);
    $b = Fiber::suspend('second');
    echo "fiber got $b, a=$a\n";
    return "done";
});
var_dump($f->isStarted());
$v = $f->start(5);
var_dump($v, $f->isSuspended(), $f->isRunning());
$v = $f->resume(100);
var_dump($v);
$v = $f->resume('last');
var_dump($v, $f->isTerminated(), $f->getReturn());

// throw into a fiber
$g = new Fiber(function () {
    try { Fiber::suspend(1); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
    return 'after';
});
$g->start();
$g->throw(new Exception('boom'));
var_dump($g->getReturn());

// errors
try { Fiber::suspend(); } catch (FiberError $e) { echo $e->getMessage(), "\n"; }
try { $f->resume(); } catch (FiberError $e) { echo $e->getMessage(), "\n"; }
try { $f->start(); } catch (FiberError $e) { echo $e->getMessage(), "\n"; }
$h = new Fiber(fn() => 1);
try { $h->getReturn(); } catch (FiberError $e) { echo $e->getMessage(), "\n"; }
try { new FiberError('x'); } catch (Error $e) { echo $e->getMessage(), "\n"; }

// nested fibers and getCurrent
var_dump(Fiber::getCurrent());
$outer = new Fiber(function () {
    $inner = new Fiber(function () { echo "inner fiber\n"; Fiber::suspend('i'); echo "inner resumed\n"; });
    var_dump($inner->start());
    var_dump(Fiber::getCurrent() !== null);
    $x = Fiber::suspend('o');
    $inner->resume();
    return $x;
});
var_dump($outer->start());
$outer->resume('X');
var_dump($outer->getReturn());

// an exception escaping the fiber
$e = new Fiber(function () { Fiber::suspend(); throw new RuntimeException('out'); });
$e->start();
try { $e->resume(); } catch (RuntimeException $x) { echo "escaped ", $x->getMessage(), "\n"; }
var_dump($e->isTerminated());

// a generator-free loop of suspends
$counter = new Fiber(function () { for ($i = 0; $i < 3; $i++) { Fiber::suspend($i); } });
$vals = [$counter->start()];
while (!$counter->isTerminated()) { $vals[] = $counter->resume(); }
var_dump($vals);
