<?php
// chown/chgrp/lchown/lchgrp by number or by name; false with the system's
// reason when the file is missing; a name that is no user warns. Also
// __sleep's names: in their order, public then the class's private then
// protected, a missing one warned and left out; unserialize() puts an
// unmangled name in the declared property of that name.
$f = __DIR__ . '/chown_family.php';
var_dump(chown($f, fileowner($f)), chgrp($f, filegroup($f)), lchown($f, fileowner($f)), lchgrp($f, filegroup($f)));
var_dump(chown(__DIR__ . '/no-such-file', fileowner($f)));
var_dump(chown($f, 'no-such-user-p8'), chgrp($f, 'no-such-group-p8'));
class P { private $x = 'P'; }
class C extends P {
    public $a = 1; protected $b = 2; private $c = 3;
    function __sleep() { return ['c', 'a', 'nope', 'x', 'b', "\0P\0x", 'dyn']; }
    function b() { return $this->b; }
}
$o = new C; $o->dyn = 4;
echo str_replace("\0", '~', serialize($o)), "\n";
$u = unserialize('O:1:"C":3:{s:1:"a";i:10;s:1:"b";i:20;s:1:"c";i:30;}');
var_dump($u->a, $u->b(), array_key_exists('b', get_object_vars($u)));
