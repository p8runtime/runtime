<?php
// A closure bound to an object with no scope given gets the dummy scope
// Closure (zend_create_closure_ex): its name's class and its self. A
// closure a builtin calls with too few arguments (Closure::call,
// array_map) has its own frame, from no file, on top of the trace.
class M {
    private $x = 1;
}

function show(ArgumentCountError $e): void {
    echo $e->getMessage(), "\n";
    foreach ($e->getTrace() as $i => $f) {
        echo "  #$i ", $f['class'] ?? '-', " ", $f['type'] ?? '-', " ", $f['function'], " ",
            isset($f['file']) ? basename($f['file']) . ':' . $f['line'] : '[internal]', " args ", count($f['args'] ?? []), "\n";
    }
}

$k = function ($a) {};
try { $k->bindTo(new stdClass)(); } catch (ArgumentCountError $e) { show($e); }
try { Closure::bind($k, new stdClass, null)(); } catch (ArgumentCountError $e) { show($e); }
try { $k->bindTo(new stdClass, 'static')(); } catch (ArgumentCountError $e) { show($e); }
try { $k->bindTo(null)(); } catch (ArgumentCountError $e) { show($e); }
// Unbound again, the scope stays.
try { $k->bindTo(new stdClass)->bindTo(null)(); } catch (ArgumentCountError $e) { show($e); }
try { $k->bindTo(new M, M::class)(); } catch (ArgumentCountError $e) { show($e); }

$r = new ReflectionFunction($k->bindTo(new M));
var_dump($r->getClosureScopeClass()?->name, $r->getClosureCalledClass()?->name, $r->getClosureThis()::class);
var_dump((new ReflectionFunction($k))->getClosureScopeClass()?->name);
$p = function () { return $this->x; };
try { var_dump($p->bindTo(new M)()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($p->bindTo(new M, M::class)());
$st = static function ($a) {};
try { Closure::bind($st, null, null)(); } catch (ArgumentCountError $e) { show($e); }

echo "-- builtin callers\n";
$k2 = function ($a) {};
try { $k2->call(new M); } catch (ArgumentCountError $e) { show($e); }
try { $k2->call(new M); } catch (ArgumentCountError $e) { echo $e->getTraceAsString(), "\n"; }
try { array_map(function ($a, $b) {}, [1]); } catch (ArgumentCountError $e) { show($e); }
try { array_map($k2->bindTo(new M), [], [1]); } catch (ArgumentCountError $e) { show($e); }
try { usort($arr, function ($a, $b, $c) {}); } catch (Throwable $e) { echo get_class($e), "\n"; }
$arr = [2, 1];
try { usort($arr, function ($a, $b, $c) {}); } catch (ArgumentCountError $e) { show($e); }
function f2($a, $b) {}
try { call_user_func('f2', 1); } catch (ArgumentCountError $e) { show($e); }
try { array_map('f2', [1]); } catch (ArgumentCountError $e) { show($e); }
class C { function __construct($a) {} function m($a, $b) {} }
try { (new ReflectionClass('C'))->newInstance(); } catch (ArgumentCountError $e) { show($e); }
try { (new ReflectionMethod('C', 'm'))->invoke(new C(1), 1); } catch (ArgumentCountError $e) { show($e); }
try { array_map([new C(1), 'm'], [1]); } catch (ArgumentCountError $e) { show($e); }
try { $k2(); } catch (ArgumentCountError $e) { show($e); }
// After a caught one, a later error's trace has no stale frame.
try { array_map(function ($a, $b) {}, [1]); } catch (ArgumentCountError $e) {}
try { intdiv(1, 0); } catch (DivisionByZeroError $e) { echo $e->getTraceAsString(), "\n"; }
function thrower() { throw new LogicException('x'); }
try { thrower(); } catch (LogicException $e) { echo $e->getTraceAsString(), "\n"; }
$k3 = function (int $a) { return $a; };
try { $k3->call(new M, 'x'); } catch (TypeError $e) { echo $e->getTraceAsString(), "\n"; }

// self in such a closure is Closure (last: without the scope it is an error).
$s = function () { return [self::class, static::class, get_class($this), get_called_class()]; };
var_dump($s->bindTo(new M)());
