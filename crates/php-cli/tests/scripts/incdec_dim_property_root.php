<?php
// ++/-- on an element of a property or a static property's array.
class A {
    public $f = ['n' => 1];
    public static $s = ['k' => 1];
    function go() {
        ++$this->f['n'];
        $this->f['n']++;
        --$this->f['n'];
        ++$this->f['m'];
        var_dump($this->f, $this->f['n']--, $this->f['n']);
        ++self::$s['k'];
        self::$s['k']--;
        ++A::$s['k'];
        var_dump(self::$s);
    }
}
(new A)->go();
$o = new stdClass;
$o->a = ['x' => ['y' => 1]];
++$o->a['x']['y'];
var_dump($o->a['x']['y']++, $o->a);
