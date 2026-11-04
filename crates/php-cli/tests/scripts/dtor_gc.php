<?php
// Destructors of objects held only by a garbage cycle, in the order the
// cycle's members held them, when gc_collect_cycles() frees it.
class D {
    function __construct(public $n) {}
    function __destruct() { echo "d{$this->n} "; }
}
echo "plain cycle: ";
$g1 = new stdClass; $g2 = new stdClass; $g1->o = $g2; $g2->o = $g1;
$g1->d = new D('g1'); $g2->d = new D('g2'); $g1->list = [new D('l1'), new D('l2')];
$g1 = $g2 = null; echo "| "; gc_collect_cycles(); echo "\n";
echo "end\n";
