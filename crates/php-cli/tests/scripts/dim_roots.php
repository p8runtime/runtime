<?php
class C {
    public static $a = [];
    public static $m = ['x' => ['y' => 1]];
    public $arr = [3, 1, 2];
    static function add($k, $v) { self::$a[$k] = $v; static::$a[] = "$k!"; }
    function first() { return reset($this->arr); }
    function next() { return next($this->arr); }
    function push($v) { array_push($this->arr, $v); return count($this->arr); }
    function sorted() { sort($this->arr); return $this->arr; }
}
C::add('k', 1);
C::$a['n'] ??= 5;
C::$a['n'] += 10;
C::$m['x']['y']++;
C::$m['x']['z'][] = 'deep';
var_dump(C::$a, C::$m, isset(C::$a['k']), isset(C::$a['nope']), isset(C::$m['x']['y']));
unset(C::$a['k']);
var_dump(array_keys(C::$a));
$c = new C;
var_dump($c->first(), $c->next(), $c->push(9), $c->sorted());
$o = new stdClass; $o->list = [5];
array_push($o->list, 6);
$st = [2, 1];
$o->n = ['b' => [3, 2, 1]];
sort($o->n['b']);
var_dump($o);
function f() { return [1, 2]; }
f()[0] = 9;
var_dump(isset(f()[1]), isset(f()[5]));
function g() { return new ArrayObject([]); }
$ao = new ArrayObject([]);
function h() { global $ao; return $ao; }
h()['set'] = 'via call';
var_dump($ao['set']);
class AA implements ArrayAccess {
    private $d = [];
    function offsetExists($o): bool { return isset($this->d[$o]); }
    function offsetGet($o): mixed { return $this->d[$o]; }
    function offsetSet($o, $v): void { $this->d[$o] = $v; }
    function offsetUnset($o): void { unset($this->d[$o]); }
    function go() { $this['bar'] = 'baz'; return [isset($this['bar']), $this['bar']]; }
}
var_dump((new AA)->go());
function byRef(&$x) { $x = 'changed'; }
class S { static $v = 'orig'; }
byRef(S::$v);
var_dump(S::$v);
