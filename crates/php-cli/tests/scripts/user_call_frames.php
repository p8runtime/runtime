<?php
// call_user_func(callback, ...) and call_user_func_array(callback, args)
// named at compile time are ZEND_INIT_USER_CALL: the callback is the
// caller's own call, with no call_user_func frame in a backtrace or an
// exception's trace. Any other call of them (by a variable's name, with
// `...` or a named argument) is the builtin's, and its frame shows.
function frames() {
    $out = [];
    foreach (debug_backtrace() as $f) {
        $out[] = (isset($f['class']) ? $f['class'] . $f['type'] : '') . $f['function'] . (isset($f['line']) ? '@' . $f['line'] : '@-') . (isset($f['file']) ? '' : '(no file)');
    }
    return implode(' < ', $out);
}
function show() { echo frames(), "\n"; debug_print_backtrace(); return func_num_args(); }
function thrower($a) { throw new Exception("thrown $a"); }
function trace(callable $run) {
    try { $run(); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
}
class C {
    public $tag = 'c';
    function m($a) { show($a); return $this->tag . $a; }
    static function s($a) { show($a); return "s$a"; }
    function viaThis() { return call_user_func([$this, 'm'], 2); }
    static function viaStatic() { return call_user_func('static::s', 3); }
}
class Hooks {
    private $callbacks = [];
    function add($cb) { $this->callbacks[] = $cb; }
    function apply($value) {
        foreach ($this->callbacks as $cb) { $value = call_user_func_array($cb, [$value]); }
        return $value;
    }
}
function filter_up($v) { echo frames(), "\n"; return strtoupper($v); }

echo "-- names\n";
var_dump(call_user_func('show', 1, 2));
var_dump(call_user_func_array('show', [1, 2, 3]));
var_dump(call_user_func('SHOW'));
var_dump(call_user_func('\show'));
echo "-- methods\n";
$c = new C;
var_dump(call_user_func([$c, 'm'], 1));
var_dump(call_user_func_array([$c, 'm'], [1]));
var_dump(call_user_func('C::s', 1));
var_dump(call_user_func_array([C::class, 's'], [1]));
var_dump($c->viaThis(), C::viaStatic());
echo "-- closures\n";
$k = 7;
var_dump(call_user_func(function ($a) use ($k) { show($a); return $a + $k; }, 1));
var_dump(call_user_func_array(fn($a) => show($a), [5]));
var_dump(call_user_func(show(...), 9));
echo "-- nested\n";
function outer() { return call_user_func('inner_fn'); }
function inner_fn() { return call_user_func_array('show', ['in']); }
var_dump(call_user_func('outer'));
echo "-- hooks\n";
$h = new Hooks;
$h->add('filter_up');
$h->add([$c, 'm']);
$h->add(fn($v) => "[$v]");
var_dump($h->apply('x'));
echo "-- builtins\n";
var_dump(call_user_func('strtoupper', 'abc'), call_user_func_array('str_repeat', ['ab', 2]));
trace(fn() => call_user_func('array_chunk', [1], 0));
trace(fn() => call_user_func_array('array_chunk', [[1], 0]));
echo "-- exceptions\n";
trace(fn() => call_user_func('thrower', 1));
trace(fn() => call_user_func_array('thrower', [2]));
trace(fn() => call_user_func([new C, 'm']));
try { call_user_func('thrower', 3); } catch (Exception $e) { echo $e->getTraceAsString(), "\n"; }
echo "-- errors\n";
try { call_user_func('nope'); } catch (TypeError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
try { call_user_func_array('show', 3); } catch (TypeError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
try { call_user_func_array('nope', 3); } catch (TypeError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
try { call_user_func([$c, 'nope']); } catch (TypeError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
try { call_user_func('thrower'); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
try { call_user_func(); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
try { call_user_func_array('show'); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
echo "-- by reference\n";
function bump(&$x) { $x++; echo frames(), "\n"; return $x; }
$v = 1;
var_dump(call_user_func('bump', $v), $v);
$list = [&$v];
var_dump(call_user_func_array('bump', $list), $v);
echo "-- the builtin's frame\n";
$f = 'call_user_func';
var_dump($f('show', 1));
$a = ['show', 2];
var_dump(call_user_func(...$a));
var_dump(call_user_func(callback: 'show'));
$g = 'call_user_func_array';
var_dump($g('show', [3]));
trace(fn() => $f('thrower', 4));
echo "-- generators and arguments\n";
function gen($n) { for ($i = 0; $i < $n; $i++) { yield $i; } }
foreach (call_user_func('gen', 3) as $x) { echo $x; }
echo "\n";
function all_args() { return func_get_args(); }
echo json_encode(call_user_func('all_args', 1, 2, 3)), json_encode(call_user_func_array('all_args', [4, 5])), "\n";
function fact($n) { return $n <= 1 ? 1 : $n * call_user_func('fact', $n - 1); }
echo fact(10), "\n";
