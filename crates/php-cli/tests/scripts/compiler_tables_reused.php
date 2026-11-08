<?php
// The compiler keeps one function's name tables (constants, variables,
// call sites, class refs) for the next function, emptied: a literal, a
// variable, a call or a class the earlier function also named must be
// the later function's own.

function first($a) {
    $x = "shared";
    $y = 42;
    $n = strlen($x);
    $o = new ArrayObject([$a]);
    return [$x, $y, $n, count($o), $a];
}

function second($b) {
    $y = "shared";
    $x = 42;
    $m = count([$b]);
    $o = new ArrayObject([1, 2]);
    return [$y, $x, $m, strlen("shared"), $b];
}

function third() {
    $z = 42;
    return "shared" . $z;
}

var_dump(first(1), second(2), third());

$c = function ($p) {
    $x = "shared";
    return $x . $p . strlen($x);
};
var_dump($c("!"));

class K {
    const C = "shared";
    public static function s() { return self::C . strlen(self::C); }
    public function m() { $x = "shared"; return $x . static::C; }
}
var_dump(K::s(), (new K)->m(), first(3)[0]);
