<?php
// `$o->m($a, 1, $b)`: a method call whose arguments are plain variables
// and literals finds its method first and sends the variables after, each
// as the method that runs takes it (INIT_METHOD_CALL, then the sends).

set_error_handler(function ($no, $msg) { echo "warning: $msg\n"; return true; });

// 1. By value and by reference in one call, with literals between; the
// receiver's class changing at the site.
class Mix {
    function mix($a, $one, &$b, $lit, &$c) { $a .= '!'; $b .= "$one"; $c .= $lit; return 'Mix'; }
}
class MixTwo extends Mix {
    function mix($a, $one, &$b, $lit, &$c) { $b .= '-'; $c .= '-'; return 'MixTwo'; }
}
class Plain {
    function mix($a, $one, $b, $lit, $c) { $a .= '!'; $b .= '!'; $c .= '!'; return 'Plain'; }
}
[$a, $b, $c] = ['a', 'b', 'c'];
foreach ([new Mix, new Mix, new Plain, new MixTwo, new Mix, new Plain, new Plain, new MixTwo] as $o) {
    echo $o->mix($a, 1, $b, 'x', $c), " $a $b $c\n";
}

// 2. No method, no send: a receiver that is null, a method that is not
// there, a private one. The undefined variable is never read, so nothing
// warns before the error.
class Shut { private function closed($x) { return 'closed'; } }
function no_send($o) {
    try {
        return $o->closed($never_defined);
    } catch (Error $e) {
        return get_class($e) . ': ' . $e->getMessage();
    }
}
function no_method($o) {
    try {
        return $o->absent($never_defined, 2);
    } catch (Error $e) {
        return get_class($e) . ': ' . $e->getMessage();
    }
}
foreach ([null, new Shut, new Shut, 5, null] as $o) {
    echo no_send($o), "\n";
    echo no_method($o), "\n";
}

// 3. A private method and a public one of the same name that take the
// argument differently: the one that runs decides (the caller's class).
class Keeper {
    private function touch(&$x) { $x .= '+K'; return 'Keeper::touch'; }
    function viaKeeper($o, $v) { $r = $o->touch($v); return "$r $v"; }
}
class Child extends Keeper {
    public function touch($x) { $x .= '+C'; return 'Child::touch'; }
    function viaChild($o, $v) { $r = $o->touch($v); return "$r $v"; }
}
class Other {
    private function touch($x) { $x .= '+O'; return 'Other::touch'; }
    function viaOther($o, $v) { $r = $o->touch($v); return "$r $v"; }
}
class OtherChild extends Other {
    public function touch(&$x) { $x .= '+OC'; return 'OtherChild::touch'; }
    function viaOtherChild($o, $v) { $r = $o->touch($v); return "$r $v"; }
}
$child = new Child;
$oc = new OtherChild;
for ($i = 0; $i < 3; $i++) {
    $v = "v$i";
    echo $child->viaKeeper($child, $v), ' | ', $child->viaChild($child, $v), ' | ', $child->touch($v), " $v\n";
    echo $oc->viaOther($oc, $v), ' | ', $oc->viaOtherChild($oc, $v), ' | ', $oc->touch($v), " $v\n";
}

// 4. __call takes them by value; a static method called on an object and
// a builtin's method take a reference where they declare one.
class Magic {
    function __call($name, $args) { $args[0] .= '?'; return $name . ':' . implode(',', $args); }
    static function stat(&$x, $y) { $x .= $y; return 'stat'; }
}
$m = new Magic;
$db = new PDO('sqlite::memory:');
$st = $db->prepare('SELECT ? AS v');
for ($i = 0; $i < 3; $i++) {
    $v = "v$i";
    echo $m->anything($v, $i), " $v ", $m->stat($v, $i), " $v\n";
    $st->bindParam(1, $bound);
    $bound = "bound late $i";
    $st->execute();
    echo $st->fetchColumn(), "\n";
}

// 5. A closure's own methods.
$add = function ($x, $y) { $x += $y; return $x; };
class Counter { public $n = 0; }
$bump = function ($by) { $this->n += $by; $by = 0; return $this->n; };
$counter = new Counter;
for ($i = 1; $i < 4; $i++) {
    $n = $i;
    echo $add->__invoke($n, $i), " $n ";
    $by = $i;
    echo $bump->call($counter, $by), " $by\n";
}

// 6. The receiver's own variable sent by reference; a variable that is a
// reference already; one unset through its reference.
class Swap {
    function replace(&$x) { $x = 'replaced'; return get_class($this); }
    function read($x, &$y) { $y = "$x!"; return $x; }
}
function swap_it() {
    $o = new Swap;
    $r = $o->replace($o);
    return "$r $o";
}
function with_globals() {
    global $shared, $fresh;
    $o = new Swap;
    $r = $o->read($shared, $fresh);
    return "$r $fresh";
}
function with_static() {
    static $kept = 'kept';
    $o = new Swap;
    $o->read($kept, $kept);
    return $kept;
}
$shared = 'shared';
for ($i = 0; $i < 3; $i++) {
    echo swap_it(), ' ', with_globals(), ' ', with_static(), "\n";
    unset($fresh);
}
unset($shared);
echo with_globals(), "\n";

// 7. `?->`: null calls nothing and sends nothing.
function maybe($o) {
    $v = 'v';
    $r = $o?->replace($v);
    $s = $o?->read($undefined_here, $v);
    return var_export($r, true) . ' ' . var_export($s, true) . " $v";
}
foreach ([null, new Swap, new Swap, null] as $o) {
    echo maybe($o), "\n";
}

// 8. The warning's handler runs after the method is found and before it
// runs: an exception from it is the call's, in the caller, and a handler
// that puts another object in the receiver's variable changes nothing of
// this call.
class First { function name($x) { return 'First ' . var_export($x, true); } }
class Second { function name($x) { return 'Second ' . var_export($x, true); } }
function throwing() {
    $o = new First;
    for ($i = 0; $i < 3; $i++) {
        try {
            echo $o->name($not_set), "\n";
        } catch (ErrorException $e) {
            echo 'caught: ', $e->getMessage(), "\n";
        }
    }
}
set_error_handler(function ($no, $msg) { throw new ErrorException($msg); });
throwing();
set_error_handler(function ($no, $msg) { $GLOBALS['who'] = new Second; echo "warning: $msg\n"; return true; });
$who = new First;
function replaced_receiver() {
    global $who;
    return $who->name($not_set);
}
echo replaced_receiver(), "\n";
echo replaced_receiver(), "\n";
set_error_handler(function ($no, $msg) { echo "warning: $msg\n"; return true; });

// 9. Too few arguments, a generator, more arguments than parameters.
class Few {
    function two(&$a, $b) { $a .= $b; return 'two'; }
    function gen(&$a, $n) { for ($i = 0; $i < $n; $i++) { $a .= $i; yield $i; } }
    function none() { return 'none' . func_num_args(); }
}
$f = new Few;
for ($i = 0; $i < 3; $i++) {
    $v = 'v';
    try {
        echo $f->two($v), "\n";
    } catch (ArgumentCountError $e) {
        echo get_class($e), ': ', strtok($e->getMessage(), ','), " $v\n";
    }
    echo $f->two($v, $i), " $v ", implode(',', iterator_to_array($f->gen($v, $i))), " $v ", $f->none($v, $i, $v), "\n";
}

// 10. A variable past the registers the loop keeps at hand: the same.
function wide_frame($o) {
    $a0 = 0; $a1 = 0; $a2 = 0; $a3 = 0; $a4 = 0; $a5 = 0; $a6 = 0; $a7 = 0; $a8 = 0; $a9 = 0; $a10 = 0; $a11 = 0; $a12 = 0; $a13 = 0; $a14 = 0; $a15 = 0;
    $a16 = 0; $a17 = 0; $a18 = 0; $a19 = 0; $a20 = 0; $a21 = 0; $a22 = 0; $a23 = 0; $a24 = 0; $a25 = 0; $a26 = 0; $a27 = 0; $a28 = 0; $a29 = 0; $a30 = 0; $a31 = 0;
    $a32 = 0; $a33 = 0; $a34 = 0; $a35 = 0; $a36 = 0; $a37 = 0; $a38 = 0; $a39 = 0; $a40 = 0; $a41 = 0; $a42 = 0; $a43 = 0; $a44 = 0; $a45 = 0; $a46 = 0; $a47 = 0;
    $a48 = 0; $a49 = 0; $a50 = 0; $a51 = 0; $a52 = 0; $a53 = 0; $a54 = 0; $a55 = 0; $a56 = 0; $a57 = 0; $a58 = 0; $a59 = 0; $a60 = 0; $a61 = 0; $a62 = 0; $a63 = 0;
    $a64 = 0; $a65 = 0; $a66 = 0; $a67 = 0; $a68 = 0; $a69 = 0; $a70 = 0; $a71 = 0; $a72 = 0; $a73 = 0; $a74 = 0; $a75 = 0; $a76 = 0; $a77 = 0; $a78 = 0; $a79 = 0;
    $a80 = 0; $a81 = 0; $a82 = 0; $a83 = 0; $a84 = 0; $a85 = 0; $a86 = 0; $a87 = 0; $a88 = 0; $a89 = 0; $a90 = 0; $a91 = 0; $a92 = 0; $a93 = 0; $a94 = 0; $a95 = 0;
    $a96 = 0; $a97 = 0; $a98 = 0; $a99 = 0; $a100 = 0; $a101 = 0; $a102 = 0; $a103 = 0; $a104 = 0; $a105 = 0; $a106 = 0; $a107 = 0; $a108 = 0; $a109 = 0; $a110 = 0; $a111 = 0;
    $a112 = 0; $a113 = 0; $a114 = 0; $a115 = 0; $a116 = 0; $a117 = 0; $a118 = 0; $a119 = 0; $a120 = 0; $a121 = 0; $a122 = 0; $a123 = 0; $a124 = 0; $a125 = 0; $a126 = 0; $a127 = 0;
    $a128 = 0; $a129 = 0; $a130 = 0; $a131 = 0; $a132 = 0; $a133 = 0; $a134 = 0; $a135 = 0; $a136 = 0; $a137 = 0; $a138 = 0; $a139 = 0; $a140 = 0; $a141 = 0; $a142 = 0; $a143 = 0;
    $a144 = 0; $a145 = 0; $a146 = 0; $a147 = 0; $a148 = 0; $a149 = 0; $a150 = 0; $a151 = 0; $a152 = 0; $a153 = 0; $a154 = 0; $a155 = 0; $a156 = 0; $a157 = 0; $a158 = 0; $a159 = 0;
    $a160 = 0; $a161 = 0; $a162 = 0; $a163 = 0; $a164 = 0; $a165 = 0; $a166 = 0; $a167 = 0; $a168 = 0; $a169 = 0; $a170 = 0; $a171 = 0; $a172 = 0; $a173 = 0; $a174 = 0; $a175 = 0;
    $a176 = 0; $a177 = 0; $a178 = 0; $a179 = 0; $a180 = 0; $a181 = 0; $a182 = 0; $a183 = 0; $a184 = 0; $a185 = 0; $a186 = 0; $a187 = 0; $a188 = 0; $a189 = 0; $a190 = 0; $a191 = 0;
    $a192 = 0; $a193 = 0; $a194 = 0; $a195 = 0; $a196 = 0; $a197 = 0; $a198 = 0; $a199 = 0; $a200 = 0; $a201 = 0; $a202 = 0; $a203 = 0; $a204 = 0; $a205 = 0; $a206 = 0; $a207 = 0;
    $a208 = 0; $a209 = 0; $a210 = 0; $a211 = 0; $a212 = 0; $a213 = 0; $a214 = 0; $a215 = 0; $a216 = 0; $a217 = 0; $a218 = 0; $a219 = 0; $a220 = 0; $a221 = 0; $a222 = 0; $a223 = 0;
    $a224 = 0; $a225 = 0; $a226 = 0; $a227 = 0; $a228 = 0; $a229 = 0; $a230 = 0; $a231 = 0; $a232 = 0; $a233 = 0; $a234 = 0; $a235 = 0; $a236 = 0; $a237 = 0; $a238 = 0; $a239 = 0;
    $a240 = 0; $a241 = 0; $a242 = 0; $a243 = 0; $a244 = 0; $a245 = 0; $a246 = 0; $a247 = 0; $a248 = 0; $a249 = 0; $a250 = 0; $a251 = 0; $a252 = 0; $a253 = 0; $a254 = 0; $a255 = 0;
    $a256 = 0; $a257 = 0; $a258 = 0; $a259 = 0; $far = 'far'; $near = 'near';
    $r = '';
    for ($i = 0; $i < 3; $i++) {
        $r .= $o->mix($a0, 1, $far, $i, $near) . ' ';
    }
    return "$r$far $near $a0 $a255 $a259";
}
echo wide_frame(new Mix), "\n";
echo "done\n";
