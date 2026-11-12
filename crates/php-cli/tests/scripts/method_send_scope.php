<?php
// A method call sends its variables before it runs (the sends are
// instructions of their own). Each send asks whether the method the
// call WILL run takes that argument by reference: the first time the
// site runs, and for a class or a calling class it has not seen, that is
// found by name, with the caller's class in mind.

// 1. The calling class's own private method comes before a subclass's
// public one of the same name. Each site below runs once per object.
class Keeper {
    private function touch(&$x, $n) { $x .= "+K$n"; return 'Keeper::touch'; }
    function viaKeeper($o, $v, $n) { $r = $o->touch($v, $n + 1); return "$r $v"; }
    function elemKeeper($o, $a, $n) { $r = $o->touch($a['k'], $n + 1); return "$r " . json_encode($a); }
}
class Child extends Keeper {
    public function touch($x, $n) { $x .= "+C$n"; return 'Child::touch'; }
    function viaChild($o, $v, $n) { $r = $o->touch($v, $n + 1); return "$r $v"; }
    function elemChild($o, $a, $n) { $r = $o->touch($a['k'], $n + 1); return "$r " . json_encode($a); }
}
class Other {
    private function touch($x, $n) { $x .= "+O$n"; return 'Other::touch'; }
    function viaOther($o, $v, $n) { $r = $o->touch($v, $n + 1); return "$r $v"; }
    function elemOther($o, $a, $n) { $r = $o->touch($a['k'], $n + 1); return "$r " . json_encode($a); }
}
class OtherChild extends Other {
    public function touch(&$x, $n) { $x .= "+OC$n"; return 'OtherChild::touch'; }
    function viaOtherChild($o, $v, $n) { $r = $o->touch($v, $n + 1); return "$r $v"; }
    function elemOtherChild($o, $a, $n) { $r = $o->touch($a['k'], $n + 1); return "$r " . json_encode($a); }
}
$child = new Child;
$oc = new OtherChild;
echo $child->viaKeeper($child, 'v', 0), "\n";
echo $child->viaChild($child, 'v', 0), "\n";
echo $child->elemKeeper($child, ['k' => 'e'], 0), "\n";
echo $child->elemChild($child, ['k' => 'e'], 0), "\n";
echo $oc->viaOther($oc, 'v', 0), "\n";
echo $oc->viaOtherChild($oc, 'v', 0), "\n";
echo $oc->elemOther($oc, ['k' => 'e'], 0), "\n";
echo $oc->elemOtherChild($oc, ['k' => 'e'], 0), "\n";
// An element that is not there: made (null) for a reference, a warning
// for a value.
echo $child->elemKeeper($child, [], 1), "\n";
echo $oc->elemOtherChild($oc, [], 1), "\n";
echo $oc->elemOther($oc, [], 1), "\n";

// The same site, a second calling class: the site saw the first.
class Base {
    private function mark(&$x, $n) { $x .= "+B$n"; return 'Base::mark'; }
    function call($o, $v, $n) { $r = $o->mark($v, $n + 1); return "$r $v"; }
}
class Sub extends Base {
    public function mark($x, $n) { $x .= "+S$n"; return 'Sub::mark'; }
}
function outside($o, $v, $n) { $r = $o->mark($v, $n + 1); return "$r $v"; }
$sub = new Sub;
for ($i = 0; $i < 3; $i++) {
    echo $sub->call($sub, 'v', $i), ' | ', outside($sub, 'v', $i), "\n";
}

// 2. A method the caller may not see, in a class with __call: __call
// answers, and it takes its arguments by value.
class Hidden {
    private function secret(&$x, $n) { $x .= "+secret$n"; return 'Hidden::secret'; }
    protected function guarded(&$x, $n) { $x .= "+guarded$n"; return 'Hidden::guarded'; }
    function __call($name, $args) { $args[0] .= '?'; return "__call $name " . implode(',', $args); }
    function inside($v, $n) { $r = $this->secret($v, $n + 1); $s = $this->guarded($v, $n + 1); return "$r $s $v"; }
    function onOther($o, $v, $n) { $r = $o->secret($v, $n + 1); $s = $o->guarded($v, $n + 1); return "$r $s $v"; }
}
class HiddenChild extends Hidden {
    function fromChild($o, $v, $n) { $r = $o->secret($v, $n + 1); $s = $o->guarded($v, $n + 1); return "$r $s $v"; }
}
function from_outside($o, $v, $a, $n) {
    $r = $o->secret($v, $n + 1);
    $s = $o->guarded($v, $n + 1);
    $t = $o->secret($a['k'], $n + 1);
    $u = $o->guarded($a['none'], $n + 1);
    return "$r | $s | $t | $u | $v " . json_encode($a);
}
$h = new Hidden;
$hc = new HiddenChild;
echo from_outside($h, 'v', ['k' => 'e'], 0), "\n";
echo $h->inside('v', 0), "\n";
echo $h->onOther($hc, 'v', 0), "\n";
echo $hc->fromChild($hc, 'v', 0), "\n";
echo $hc->fromChild($h, 'v', 0), "\n";
echo from_outside($hc, 'v', ['k' => 'e'], 1), "\n";

// 3. The same two with nothing computed among the arguments (a variable,
// a literal): the first time through each site, then again.
class Plain {
    private function f($x) { $x = 'changed'; return 'Plain::f'; }
    private function g(&$x) { $x = 'changed'; return 'Plain::g'; }
    function viaPlain($o) { $v = 'v'; $w = 'w'; $r = $o->f($v); $s = $o->g($w); return "$r $v $s $w"; }
}
class PlainChild extends Plain {
    public function f(&$x) { $x = 'child'; return 'PlainChild::f'; }
    public function g($x) { $x = 'child'; return 'PlainChild::g'; }
    function viaChild($o) { $v = 'v'; $w = 'w'; $r = $o->f($v); $s = $o->g($w); return "$r $v $s $w"; }
}
$pc = new PlainChild;
echo $pc->viaPlain($pc), "\n";
echo $pc->viaChild($pc), "\n";
echo $pc->viaPlain($pc), "\n";
echo $pc->viaPlain(new Plain), "\n";
echo $pc->viaPlain($pc), "\n";
function plain_outside($o) { $v = 'v'; $r = $o->secret($v, 1); $s = $o->guarded($v, 2); return "$r | $s | $v"; }
echo plain_outside(new Hidden), "\n";
echo plain_outside(new HiddenChild), "\n";
echo plain_outside(new Hidden), "\n";
