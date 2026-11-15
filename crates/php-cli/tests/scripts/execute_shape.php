<?php
// The interpreter loop's frame switches and rare paths: a callee's variables
// undefined whatever the caller's registers held, defaults and extra
// arguments, destructors due at a return, a callback's return to a builtin,
// a frame with shared variables (an include in a function), exceptions
// raised on the loop's out-of-line paths.

class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "destruct {$this->n}\n"; }
}

function fill() { $a = 1; $b = "two"; $c = [3]; $d = new D("fill"); return $a; }
function fresh($x) { echo isset($a) ? "a set\n" : "a unset\n"; echo isset($d) ? "d set\n" : "d unset\n"; return $x; }
fill();
echo fresh(5), "\n";

function defaults($a, $b = 2, $c = [3]) { return $a + $b + count($c); }
echo defaults(1), " ", defaults(1, 5), " ", defaults(1, 5, [1, 2]), "\n";
function extra($a) { return $a + count(func_get_args()); }
echo extra(1, 2, 3), "\n";
function variadic($a, ...$rest) { return $a . ":" . implode(",", $rest); }
echo variadic("v", 1, 2, 3), "\n";

function temp() { new D("temp"); echo "after new\n"; return 1; }
echo temp(), "\n";
function holds() { $d = new D("local"); echo "in holds\n"; return 2; }
echo holds(), "\n";
echo "after holds\n";

echo implode(",", array_map(fn($x) => $x * 2, [1, 2, 3])), "\n";
$list = [3, 1, 2];
usort($list, function ($a, $b) { $d = new D("cmp"); return $a <=> $b; });
echo implode(",", $list), "\n";

function includes() {
    $local = "before";
    $f = tempnam(sys_get_temp_dir(), "es");
    file_put_contents($f, '<?php $local = "from include"; $made = 7;');
    include $f;
    unlink($f);
    echo $local, " ", $made, "\n";
}
includes();

function thrower($n) { if ($n === 0) { throw new RuntimeException("deep"); } return thrower($n - 1); }
try { thrower(5); } catch (RuntimeException $e) { echo "caught ", $e->getMessage(), "\n"; }
function undefinedIndex() { $a = []; try { return intdiv(1, 0); } catch (DivisionByZeroError $e) { return "div"; } }
echo undefinedIndex(), "\n";
class M { public function __call($n, $a) { throw new LogicException("no $n"); } }
try { (new M)->missing(1); } catch (LogicException $e) { echo "caught ", $e->getMessage(), "\n"; }
function rec($n) { return $n <= 0 ? 0 : $n + rec($n - 1); }
echo rec(100), "\n";
echo "end\n";
