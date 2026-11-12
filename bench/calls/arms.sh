#!/usr/bin/env bash
# Where each arm of an interpreter loop starts, for lines.py --arms.
#
# Usage: bench/calls/arms.sh p8 <crates/p8-interp/src/fast.rs>
#        bench/calls/arms.sh p8-dispatch <target/.../build/p8-interp-*/out/dispatch.rs>
#        bench/calls/arms.sh php <Zend/zend_vm_execute.h>
# Prints `line<TAB>name` lines.
#
# p8: the hand arms of `execute` (`Op::X =>` in the dispatch! call), the
# frame loop head, the inner loop head, and each macro defined in either
# loop (code from a macro keeps the lines of the macro, whichever arm
# uses it).
# p8-dispatch: the generated arms (one per line) and the match itself.
# php: the HYBRID_CASE(...) labels inside execute_ex and the labels of
# the shared helpers (zend_leave_helper and the like).
set -euo pipefail
case ${1:?usage: arms.sh p8|p8-dispatch|php <file>} in
    p8)
        awk '
            /pub\(crate\) fn execute\(/ && !fn_line { fn_line = NR; print NR "\t(execute: entry)"; next }
            fn_line && !frame && /\047frame: loop \{/ { frame = NR; print NR "\t(frame head)"; next }
            frame && !done && !in_macro && /^ +macro_rules! [a-z_]+ \{/ {
                name = $0; sub(/^ */, "", name); sub(/macro_rules! /, "", name); sub(/ \{.*/, "", name)
                indent = match($0, /[^ ]/) - 1
                if (indent == 12 || indent == 16) { in_macro = indent; print NR "\t(" name "!)"; next }
            }
            in_macro && match($0, /[^ ]/) - 1 == in_macro && /^ +\}$/ { print NR + 1 "\t" (in_macro == 12 && !inner ? "(frame head)" : "(loop head)"); in_macro = 0; next }
            frame && !inner && /^            loop \{/ { inner = NR; print NR "\t(loop head)"; next }
            inner && !done && !in_macro && /^                    Op::[A-Za-z0-9]+.*=>/ {
                name = $0; sub(/^ */, "", name); sub(/ *=>.*/, "", name); sub(/ if .*/, "", name)
                print NR "\t" name; next
            }
            inner && !done && /^    }$/ { print NR "\t(after execute)"; done = 1 }
        ' "$2"
        ;;
    p8-dispatch)
        awk '
            /let insn: &Insn = \$insn;/ { print NR "\t(insn!)" }
            /^ +match \$op \{/ { print NR "\t(dispatch)" }
            /^ +Op::[A-Za-z0-9_]+ =>/ { name = $1; print NR "\t" name }
        ' "$2"
        ;;
    php)
        awk '
            /^ZEND_API void execute_ex\(/ { inside = 1 }
            inside && /HYBRID_CASE\(/ { name = $0; sub(/.*HYBRID_CASE\(/, "", name); sub(/\).*/, "", name); print NR "\t" name }
            inside && /^[a-z_A-Z0-9]+_LABEL:/ { name = $0; sub(/_LABEL:.*/, "", name); print NR "\t" name }
        ' "$2"
        ;;
    *) echo "usage: arms.sh p8|p8-dispatch|php <file>" >&2; exit 2 ;;
esac
