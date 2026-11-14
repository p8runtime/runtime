<?php
// strlen($x) .. is_object($x) of a variable read where it is
// (Op::IntrinsicRegs): each intrinsic of one argument on every kind of
// value, a variable that is a reference, an undefined one (the warning,
// then the intrinsic of null), the conversions and errors of the call,
// a Countable, and a function of the same name in a namespace (the call,
// not the intrinsic). Each case runs twice, so the second run goes
// through the site the first resolved. The expected output is PHP 8.6's.

namespace {
    function show(Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    class Counted implements Countable { function count(): int { return 7; } }
    class Str { function __toString(): string { return 'str'; } }
    class Noisy { function __construct(public $n) {} function __destruct() { echo "destruct {$this->n}\n"; } }

    function kinds($v) {
        return implode(',', [var_export(is_null($v), true), var_export(is_bool($v), true), var_export(is_int($v), true),
            var_export(is_float($v), true), var_export(is_string($v), true), var_export(is_array($v), true), var_export(is_object($v), true)]);
    }

    function lengths() {
        $s = "hello world"; $e = ""; $u = "€uro"; $n = 12345; $f = 1.5; $t = true; $z = null;
        $a = [1, 2, 3]; $h = ['x' => 1]; $o = new Counted; $str = new Str;
        echo strlen($s), ' ', strlen($e), ' ', strlen($u), ' ', strlen($n), ' ', strlen($f), ' ', strlen($t), ' ', strlen($str), "\n";
        echo count($a), ' ', count($h), ' ', count($o), "\n";
        echo @strlen($z), "\n";
        try { echo strlen($a), "\n"; } catch (Throwable $e) { show($e); }
        try { echo count($n), "\n"; } catch (Throwable $e) { show($e); }
        try { echo count($str), "\n"; } catch (Throwable $e) { show($e); }
        foreach ([$s, $n, $f, $t, $z, $a, $o, STDIN] as $v) {
            echo kinds($v), "\n";
        }
        $t2 = 0;
        for ($i = 0; $i < 5; $i++) {
            $t2 += strlen($s) + count($a);
        }
        echo $t2, "\n";
    }

    function references() {
        $s = "abc"; $r = &$s; $a = [1]; $ra = &$a;
        echo strlen($r), ' ', count($ra), ' ', var_export(is_string($r), true), ' ', var_export(is_array($ra), true), "\n";
        $r .= "d"; $ra[] = 2;
        echo strlen($s), ' ', count($a), "\n";
    }

    function undefined() {
        echo var_export(is_null($nope), true), "\n";
        echo strlen($none), "\n";
        echo count($missing ?? []), "\n";
    }

    function released() {
        // The result goes where the argument went: an object the register
        // held last is released by the next write there, as without the
        // fused form.
        $o = new Noisy(1);
        echo var_export(is_object($o), true), "\n";
        $o = null;
        echo "after\n";
    }

    for ($run = 0; $run < 2; $run++) {
        echo "-- run $run\n";
        lengths();
        references();
        undefined();
        released();
        \N\shadowed("xyz");
    }
}

namespace N {
    function strlen($s) { return "N\\strlen($s)"; }
    function count($a) { return "N\\count"; }
    function shadowed($s) {
        $a = [1, 2];
        echo strlen($s), ' ', count($a), ' ', \strlen($s), ' ', \count($a), ' ', is_string($s) ? 'yes' : 'no', "\n";
    }
}
