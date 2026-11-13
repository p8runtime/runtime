<?php
// send() and throw() into a generator that is inside `yield from` (#503):
// they go to the innermost generator, where it stopped.

echo "-- the issue's script\n";
function inner() { echo "inner got: ", yield 1, "\n"; }
function outer() { yield from inner(); }
$g = outer();
$g->current();
$g->send("sent");

echo "-- several sends, keys and the result of yield from\n";
function counter() {
    $total = 0;
    while (($n = yield $total) !== null) {
        echo "counter got $n\n";
        $total += $n;
    }
    return $total;
}
function wrap() {
    echo "before\n";
    $r = yield from counter();
    echo "yield from gave $r\n";
    $last = yield "after";
    echo "wrap got $last\n";
    return $r;
}
$g = wrap();
var_dump($g->current());
var_dump($g->send(2));
var_dump($g->send(3));
var_dump($g->key(), $g->current());
var_dump($g->send(null));
var_dump($g->send("end"));
var_dump($g->valid(), $g->getReturn());

echo "-- send() first: the generator runs to its first yield, then gets it\n";
$g = wrap();
var_dump($g->send(5));
var_dump($g->send(null));

echo "-- three levels\n";
function leaf() {
    $a = yield "leaf 1";
    echo "leaf got $a\n";
    $b = yield "leaf 2";
    echo "leaf got $b\n";
    return "leaf done";
}
function middle() {
    $r = yield from leaf();
    echo "middle: $r\n";
    $c = yield "middle 1";
    echo "middle got $c\n";
    return "middle done";
}
function top() {
    $r = yield from middle();
    echo "top: $r\n";
}
$g = top();
foreach (["x", "y", "z", "w"] as $v) {
    echo "current: ", var_export($g->current(), true), "\n";
    $g->send($v);
}
var_dump($g->valid());

echo "-- next() sends null\n";
$g = outer();
$g->current();
$g->next();

echo "-- arrays and iterators ignore what is sent\n";
function from_array() {
    $r = yield from [10, 20];
    var_dump($r);
    echo "from_array got ", var_export(yield 30, true), "\n";
}
$g = from_array();
var_dump($g->current(), $g->send("a"), $g->send("b"));
$g->send("c");
function from_iterator() {
    yield from new ArrayIterator(["k" => 1, "l" => 2]);
    echo "from_iterator got ", var_export(yield 3, true), "\n";
}
$g = from_iterator();
var_dump($g->current(), $g->send("a"), $g->send("b"));
$g->send("c");

echo "-- throw() goes to the innermost generator\n";
function catcher() {
    try {
        yield 1;
    } catch (Exception $e) {
        echo "catcher caught ", $e->getMessage(), "\n";
        $v = yield 2;
        echo "catcher got $v\n";
    }
    return "catcher done";
}
function delegating() {
    $r = yield from catcher();
    echo "delegating: $r\n";
    yield 3;
}
$g = delegating();
$g->current();
var_dump($g->throw(new Exception("one")));
var_dump($g->send("after throw"));

echo "-- not caught inside: it comes out of yield from\n";
function plain() { yield 1; yield 2; }
function guarded() {
    try {
        yield from plain();
    } catch (Exception $e) {
        echo "guarded caught ", $e->getMessage(), "\n";
    }
    yield "guarded 2";
}
$g = guarded();
$g->current();
var_dump($g->throw(new Exception("two")));
$g->next();
var_dump($g->valid());

echo "-- a new yield from after the caught one\n";
function again() {
    try {
        yield from plain();
    } catch (Exception $e) {
        echo "again caught ", $e->getMessage(), "\n";
    }
    yield from plain();
}
$g = again();
$g->current();
$g->throw(new Exception("three"));
for (; $g->valid(); $g->next()) {
    echo $g->key(), " => ", $g->current(), "\n";
}

echo "-- throw() into a yield from an array\n";
function array_guarded() {
    try {
        yield from [1, 2];
    } catch (Exception $e) {
        echo "array_guarded caught ", $e->getMessage(), "\n";
    }
    yield "array_guarded 2";
}
$g = array_guarded();
$g->current();
var_dump($g->throw(new Exception("four")));

echo "-- not caught anywhere\n";
$g = outer();
$g->current();
try {
    $g->throw(new Exception("five"));
} catch (Exception $e) {
    echo "caller caught ", $e->getMessage(), "\n";
}
var_dump($g->valid());

echo "-- the inner generator is one we also hold\n";
$in = counter();
function holder($in) { return yield from $in; }
$g = holder($in);
$g->current();
$g->send(4);
var_dump($in->current(), $g->current());
$in->send(6);
var_dump($in->current(), $g->current());
$g->send(null);
var_dump($g->getReturn());

echo "done\n";
