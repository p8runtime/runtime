#!/usr/bin/env bash
# The call variants measured for #413, written as bench/count cases.
#
# Usage: bench/calls/gen.sh <dir>
#   writes <dir>/cases/*.php (one loop iteration = one call; argv[1] = the
#   iteration count, as bench/count's cases take it) and <dir>/first/*.php
#   (first call against repeated call of a call site, see the end).
#
# Every case is the same loop around one statement, so the difference
# between two cases is the cost of the one thing that differs. The body
# is `return 1;` unless the case is about the body.
set -euo pipefail
out=${1:?usage: gen.sh <dir>}
mkdir -p "$out/cases" "$out/first"

# loop <name> <declarations> <setup in run()> <the statement>
loop() {
    cat >"$out/cases/$1.php" <<EOF
<?php
\$n = (int) (\$argv[1] ?? 1000);
$2
function run(\$n) { $3 \$s = 0; for (\$i = 0; \$i < \$n; \$i++) { $4 } return \$s; }
echo run(\$n), "\n";
EOF
}

# inside <name> <class body beside run()> <the statement>: the loop is a
# method of C, so the call is made from inside the class.
inside() {
    cat >"$out/cases/$1.php" <<EOF
<?php
\$n = (int) (\$argv[1] ?? 1000);
class C { $2
  function run(\$n) { \$s = 0; for (\$i = 0; \$i < \$n; \$i++) { $3 } return \$s; } }
echo (new C)->run(\$n), "\n";
EOF
}

# ---- the loop alone, and the two statements around a call ----------------
loop base_loop '' '' ''
loop base_assign_const '' '' '$s = 1;'

# ---- functions ------------------------------------------------------------
loop fn_0 'function f() { return 1; }' '' '$s = f();'
loop fn_0_discard 'function f() { return 1; }' '' 'f();'
loop fn_0_void 'function f() { }' '' 'f();'
loop fn_1var 'function f($a) { return 1; }' '' '$s = f($i);'
loop fn_1const 'function f($a) { return 1; }' '' '$s = f(7);'
loop fn_2var 'function f($a, $b) { return 1; }' '' '$s = f($i, $i);'
loop fn_3var 'function f($a, $b, $c) { return 1; }' '' '$s = f($i, $i, $i);'
loop fn_3const 'function f($a, $b, $c) { return 1; }' '' '$s = f(7, 8, 9);'
loop fn_1var_tparam 'function f(int $a) { return 1; }' '' '$s = f($i);'
loop fn_3var_tparam 'function f(int $a, int $b, int $c) { return 1; }' '' '$s = f($i, $i, $i);'
loop fn_1var_tret 'function f($a): int { return 1; }' '' '$s = f($i);'
loop fn_1var_tboth 'function f(int $a): int { return 1; }' '' '$s = f($i);'
loop fn_1var_discard 'function f($a) { return 1; }' '' 'f($i);'
loop fn_1var_retarg 'function f($a) { return $a; }' '' '$s = f($i);'
loop fn_1var_retarg_tret 'function f($a): int { return $a; }' '' '$s = f($i);'
loop fn_1var_retarg_tboth 'function f(int $a): int { return $a; }' '' '$s = f($i);'
loop fn_2var_add 'function f($a, $b) { return $a + $b; }' '' '$s = f($i, $i);'

# ---- methods, called from outside the class ($o->m()) ----------------------
loop m_0 'class C { function m() { return 1; } }' '$o = new C;' '$s = $o->m();'
loop m_0_discard 'class C { function m() { return 1; } }' '$o = new C;' '$o->m();'
loop m_1var 'class C { function m($a) { return 1; } }' '$o = new C;' '$s = $o->m($i);'
loop m_1const 'class C { function m($a) { return 1; } }' '$o = new C;' '$s = $o->m(7);'
loop m_3var 'class C { function m($a, $b, $c) { return 1; } }' '$o = new C;' '$s = $o->m($i, $i, $i);'
loop m_3const 'class C { function m($a, $b, $c) { return 1; } }' '$o = new C;' '$s = $o->m(7, 8, 9);'
loop m_1var_tparam 'class C { function m(int $a) { return 1; } }' '$o = new C;' '$s = $o->m($i);'
loop m_1var_tret 'class C { function m($a): int { return 1; } }' '$o = new C;' '$s = $o->m($i);'
loop m_1var_discard 'class C { function m($a) { return 1; } }' '$o = new C;' '$o->m($i);'
loop m_1var_final 'final class C { function m($a) { return 1; } }' '$o = new C;' '$s = $o->m($i);'
loop m_1var_retarg 'class C { function m($a) { return $a; } }' '$o = new C;' '$s = $o->m($i);'
loop m_1var_retarg_tret 'class C { function m($a): int { return $a; } }' '$o = new C;' '$s = $o->m($i);'
loop m_1var_propread 'class C { private $x = 1; function m($a) { return $this->x; } }' '$o = new C;' '$s = $o->m($i);'
loop m_1var_propop 'class C { private $x = 0; function m($a) { $this->x += $a; return 1; } }' '$o = new C;' '$s = $o->m($i);'
loop m_1var_static 'class C { static function m($a) { return 1; } }' '' '$s = C::m($i);'
loop m_0_static 'class C { static function m() { return 1; } }' '' '$s = C::m();'

# ---- methods, called from inside the class ----------------------------------
inside m_1var_this_public 'public function m($a) { return 1; }' '$s = $this->m($i);'
inside m_1var_this_private 'private function m($a) { return 1; }' '$s = $this->m($i);'
inside m_0_this_public 'public function m() { return 1; }' '$s = $this->m();'
inside m_1var_self_static 'static function m($a) { return 1; }' '$s = self::m($i);'
inside m_1var_late_static 'static function m($a) { return 1; }' '$s = static::m($i);'

# ---- first call against repeated call ----------------------------------------
# M call sites in a straight line in run(); <name>.0.php never calls run(),
# .1.php calls it once, .2.php twice. All three compile the same code, so
# (count(.1) - count(.0)) / M is one site's first execution (with whatever
# the engine does once per function body, which `plain` measures alone)
# and (count(.2) - count(.1)) / M its second.
M=2000
first() {
    local name=$1 decl=$2 setup=$3 stmt=$4 k
    for k in 0 1 2; do
        {
            echo '<?php'
            echo "$decl"
            echo "function run() { $setup \$s = 0;"
            for ((j = 0; j < M; j++)); do echo "$stmt"; done
            echo 'return $s; }'
            case $k in
                1) echo 'run();' ;;
                2) echo 'run(); run();' ;;
            esac
            echo 'echo "ok\n";'
        } >"$out/first/$name.$k.php"
    done
}
first plain '' '' '$s = $s + 1;'
first fn 'function f($a) { return 1; }' '' '$s = f($s);'
first method 'class C { function m($a) { return 1; } }' '$o = new C;' '$s = $o->m($s);'
first static 'class C { static function m($a) { return 1; } }' '' '$s = C::m($s);'
echo "$M" >"$out/first/M"
