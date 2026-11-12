<?php
// A method call's variable arguments go by value or by reference as the
// method that is called declares, argument by argument, also after the
// call site has run before.

class Modes {
    function mixed($a, &$b, $c, &$d) { $a .= 'a'; $b .= 'b'; $c .= 'c'; $d .= 'd'; return 'mixed'; }
    function rest($a, &...$r) { $a .= 'a'; foreach ($r as &$x) { $x .= 'r'; } return 'rest' . count($r); }
    function restVal(&$a, ...$r) { $a .= 'a'; foreach ($r as &$x) { $x .= 'r'; } return 'restVal' . count($r); }
    function extra(&$a) { $a .= 'a'; return 'extra' . func_num_args(); }
    function typed(int &$n, string $s) { $n++; $s .= '!'; return 'typed'; }
}
$o = new Modes;
for ($i = 0; $i < 3; $i++) {
    [$p, $q, $r, $s] = ['p', 'q', 'r', 's'];
    echo $o->mixed($p, $q, $r, $s), " $p $q $r $s\n";
    echo $o->rest($p, $q, $r, $s), " $p $q $r $s\n";
    echo $o->restVal($p, $q, $r, $s), " $p $q $r $s\n";
    echo $o->extra($p, $q, $r, $s), " $p $q $r $s\n";
    $n = $i;
    echo $o->typed($n, $p), " $n $p\n";
}

// The same variable twice, and a by-reference argument that is not
// defined yet (it is made, null; no warning).
class Twice { function both(&$a, &$b) { $a++; $b++; return $a + $b; } function make(&$a, $n) { $a[] = $n; return count($a); } }
$t = new Twice;
for ($i = 0; $i < 3; $i++) {
    $k = $i;
    echo $t->both($k, $k), " $k\n";
    echo $t->make($made, $i), ' ', json_encode($made), "\n";
}

// A by-value argument that is not defined: the warning, then null.
function warn_once() {
    $o = new Modes;
    for ($i = 0; $i < 2; $i++) {
        $q = 'q'; $s = 's';
        echo $o->mixed($undefined, $q, $i, $s), " $q $s\n";
    }
}
set_error_handler(function ($no, $msg) { echo "warning: $msg\n"; return true; });
warn_once();
restore_error_handler();

// More parameters than the call site's bits: the 64th and 70th by
// reference, the others by value, and a by-reference variadic after 64.
class Wide {
    function wide($p0, $p1, $p2, $p3, $p4, $p5, $p6, $p7, $p8, $p9, $p10, $p11, $p12, $p13, $p14, $p15,
        $p16, $p17, $p18, $p19, $p20, $p21, $p22, $p23, $p24, $p25, $p26, $p27, $p28, $p29, $p30, $p31,
        $p32, $p33, $p34, $p35, $p36, $p37, $p38, $p39, $p40, $p41, $p42, $p43, $p44, $p45, $p46, $p47,
        $p48, $p49, $p50, $p51, $p52, $p53, $p54, $p55, $p56, $p57, $p58, $p59, $p60, $p61, &$p62, $p63,
        &$p64, $p65, $p66, $p67, $p68, &$p69) {
        $p61 .= '+'; $p62 .= '+'; $p63 .= '+'; $p64 .= '+'; $p65 .= '+'; $p69 .= '+';
        return 'wide';
    }
    function edge($p0, $p1, $p2, $p3, $p4, $p5, $p6, $p7, $p8, $p9, $p10, $p11, $p12, $p13, $p14, $p15,
        $p16, $p17, $p18, $p19, $p20, $p21, $p22, $p23, $p24, $p25, $p26, $p27, $p28, $p29, $p30, $p31,
        $p32, $p33, $p34, $p35, $p36, $p37, $p38, $p39, $p40, $p41, $p42, $p43, $p44, $p45, $p46, $p47,
        $p48, $p49, $p50, $p51, $p52, $p53, $p54, $p55, $p56, $p57, $p58, $p59, $p60, &$p61, $p62) {
        $p60 .= '+'; $p61 .= '+'; $p62 .= '+';
        return 'edge' . func_num_args();
    }
    function tail($p0, $p1, $p2, $p3, $p4, $p5, $p6, $p7, $p8, $p9, $p10, $p11, $p12, $p13, $p14, $p15,
        $p16, $p17, $p18, $p19, $p20, $p21, $p22, $p23, $p24, $p25, $p26, $p27, $p28, $p29, $p30, $p31,
        $p32, $p33, $p34, $p35, $p36, $p37, $p38, $p39, $p40, $p41, $p42, $p43, $p44, $p45, $p46, $p47,
        $p48, $p49, $p50, $p51, $p52, $p53, $p54, $p55, $p56, $p57, $p58, $p59, $p60, $p61, &...$more) {
        $p61 .= '+'; foreach ($more as &$m) { $m .= '+'; }
        return 'tail' . count($more);
    }
}
$w = new Wide;
$z = 0;
for ($i = 0; $i < 3; $i++) {
    [$a, $b, $c, $d, $e, $f] = ['a', 'b', 'c', 'd', 'e', 'f'];
    echo $w->wide($z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z,
        $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z,
        $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z,
        $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $a, $b, $c,
        $d, $e, $z, $z, $z, $f), " $a $b $c $d $e $f $z\n";
    [$a, $b, $c, $d, $e, $f] = ['a', 'b', 'c', 'd', 'e', 'f'];
    echo $w->edge($z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z,
        $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z,
        $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z,
        $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $a, $b, $c, $d, $e, $f), " $a $b $c $d $e $f $z\n";
    [$a, $b, $c, $d, $e, $f] = ['a', 'b', 'c', 'd', 'e', 'f'];
    echo $w->tail($z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z,
        $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z,
        $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z,
        $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $z, $a, $b, $c, $d, $e, $f), " $a $b $c $d $e $f $z\n";
}
echo "done\n";
