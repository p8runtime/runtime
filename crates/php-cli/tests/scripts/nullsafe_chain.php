<?php
// A null receiver of ?-> ends the whole chain (#505): nothing to its right
// runs, no property read, no index, no call, no argument.
function arg($label = "arg") { echo "$label ran\n"; return 1; }

class Box {
    public $p;
    public $none = null;
    public $list = [10, [20, 21]];
    public static $s = "static";
    public function __construct($p = null) { $this->p = $p; }
    public function m($x = null) { echo "m ran\n"; return $this; }
    public function nothing() { echo "nothing ran\n"; return null; }
    public function __get($name) { echo "__get($name)\n"; return null; }
    public static function sm($x = null) { echo "sm ran\n"; return "sm"; }
}

$n = null;
$o = new Box(new Box());

echo "-- the issue's four\n";
var_dump($n?->p->q);
var_dump($n?->m()->m(arg()));
var_dump($n?->p[0]);
var_dump($n?->p->m());

echo "-- longer chains\n";
var_dump($n?->p->q->r->s);
var_dump($n?->m(arg("a1"))->m(arg("a2"))->m(arg("a3")));
var_dump($n?->p[0][1]->q);
var_dump($n?->m()[0]);
var_dump($n?->p?->q->r);
var_dump($n?->p->list[arg("index")]);
var_dump($n?->{arg("name") ? "p" : "q"}->q);
var_dump($n?->p::$s);
var_dump($n?->p::sm(arg("static arg")));

echo "-- parentheses do not end it\n";
var_dump(($n?->p)->q);
var_dump(($n?->m())->m(arg()));

echo "-- a receiver that is not null: the chain goes on\n";
var_dump($o?->p->list[1][0]);
var_dump($o?->p?->none?->x->y);
var_dump($o?->nothing()?->m(arg("after nothing"))->m());
var_dump($o->none?->x->y);
var_dump($o?->m()->m(arg("o arg"))->list[0]);
var_dump($o?->p::$s);
var_dump($o?->p::sm(arg("o static arg")));

echo "-- a null that is not the receiver of ?->\n";
var_dump($o?->none->x);

echo "-- isset, empty, ??\n";
var_dump(isset($n?->p->q));
var_dump(isset($n?->p[0]));
var_dump(empty($n?->p->q));
var_dump($n?->p->q ?? "default");
var_dump($n?->m()->m(arg()) ?? "default after call");
var_dump($n?->p[0] ?? "default dim");
var_dump(isset($o?->p->list[1][1]));
var_dump($o?->p->list[1][1] ?? "unused");

echo "-- isset, empty, ??: no key or name to the right is evaluated\n";
var_dump(isset($n?->p[arg("isset key")]));
var_dump(isset($n?->p->{arg("isset name") ? "a" : "b"}));
var_dump(empty($n?->p[arg("empty key")]));
var_dump($n?->p[arg("coalesce key")] ?? "d");
var_dump($n?->p->{arg("coalesce name") ? "a" : "b"} ?? "d");
var_dump(isset($n?->p::$s));
var_dump(isset($n?->a, $n?->b->c));
var_dump(isset($o->p, $n?->m()->p));
var_dump(isset($o?->p->list[arg("isset key, not null")]));

echo "-- a dynamic method, a spread, a variable static property\n";
$m = "m";
var_dump($n?->p->$m(arg("dynamic method")));
var_dump($n?->p->m(...[arg("spread")]));
$k = "s";
var_dump($n?->p::$$k);
foreach ([$n?->p->q] as $v) var_dump($v);
var_dump(match ($n?->p->q) { null => "null", default => "other" });

echo "-- used as a value\n";
$x = $n?->p->q;
var_dump($x);
$a = [];
$a[] = $n?->m()->m(arg());
$a["k"] = $n?->p[0];
var_dump($a);
var_dump($n?->p->q === null);
echo "[{$n?->p->q}]\n";
echo strlen($n?->p->q ?? ""), "\n";
var_dump([$n?->p->q, $n?->m()->m(arg())]);
var_dump($n?->p->q . "tail");

echo "-- inside functions\n";
function chain($r) {
    return $r?->p->list[0];
}
function calls($r) {
    return $r?->m(arg("f1"))->m(arg("f2"))->list[1][1];
}
function dims($r, $k) {
    $v = $r?->list[$k][arg("f3")];
    return $v;
}
function tests($r) {
    return [isset($r?->p->p), $r?->p->p ?? "d", empty($r?->m()->list)];
}
var_dump(chain(null), chain($o));
var_dump(calls(null), calls($o));
var_dump(dims(null, 1), dims($o, 1));
var_dump(tests(null), tests($o));

echo "-- in a loop, null and not null\n";
foreach ([null, $o, null, new Box()] as $i => $r) {
    echo $i, ": ";
    var_dump($r?->m(arg("loop"))->p?->list[0]);
}

echo "-- a closure and a method\n";
$fn = fn($r) => $r?->p->m(arg("closure"))->list[0];
var_dump($fn(null), $fn($o));
class User {
    public function go(?Box $b) { return $b?->p->m(arg("method"))->none?->x; }
}
var_dump((new User)->go(null), (new User)->go($o));

echo "-- the chain ends, the expression around it goes on\n";
var_dump(strtoupper($n?->p->q ?? "x") . "!");
var_dump(1 + ($n?->p->q ?? 2));
var_dump(arg("before") + (int) ($n?->m(arg("never"))->q) + arg("after"));

echo "-- plain chains beside them, in the same file\n";
var_dump($o->p->list[1][0]);
var_dump($o->m(arg("plain arg"))->list[0]);
var_dump(isset($o->p->list[9]), empty($o->p->list[1]), $o->p->none ?? "plain default");
var_dump(Box::$s, Box::sm(arg("plain static")));
$plain = fn($b) => $b->p->list[1][1] ?? "none";
var_dump($plain($o));
var_dump($o->none->x ?? "null in a plain chain");

echo "done\n";
