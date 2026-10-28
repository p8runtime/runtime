<?php
// Destructors the cycle collector runs: they see globals, their
// exceptions chain, and inside a fiber each runs in a fiber of its own.
class C {
    public $self;
    function __construct(public string $name, public bool $throw = false) { $this->self = $this; }
    function __destruct() {
        global $seen, $parked;
        $seen[] = $this->name;
        if ($this->name === 'park') {
            $parked = Fiber::getCurrent();
            Fiber::suspend();
            echo "park resumed\n";
        }
        if ($this->throw) {
            throw new Exception($this->name);
        }
    }
}

$seen = [];
new C('a');
gc_collect_cycles();
echo implode(',', $seen), "\n";

new C('x', true);
new C('y', true);
try {
    gc_collect_cycles();
} catch (Exception $e) {
    echo $e->getMessage(), ' after ', $e->getPrevious()->getMessage(), "\n";
}

$f = new Fiber(function () {
    new C('park');
    gc_collect_cycles();
    echo "collected\n";
});
$f->start();
echo implode(',', $seen), "\n";
var_dump($parked instanceof Fiber, $parked === $f, $parked->isSuspended());
$parked->resume();
