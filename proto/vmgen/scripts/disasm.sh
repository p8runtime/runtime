#!/bin/sh
# The generated code of a few handlers, to check that the dead branches are
# gone and to count instructions: `disasm.sh <vmgen binary> <symbol>...`.
# Each symbol is matched as a substring of the demangled name (so
# `P8_MOVE_TMP_CONST` finds `vmgen::handlers::P8_MOVE_TMP_CONST`).
set -u
B=$1; shift
objdump -d --no-show-raw-insn -C "$B" > "${TMPDIR:-.}/vmgen-disasm.txt"
for s in "$@"; do
  awk -v s="$s" '
    /^[0-9a-f]+ <.*>:$/ { show = index($0, s) > 0 && index($0, "::" s ">") > 0; if (show) { n = 0; print; } next }
    show && /^$/ { print "  (" n " instructions)"; show = 0; next }
    show { n++; print }
  ' "${TMPDIR:-.}/vmgen-disasm.txt"
done
echo "== sizes (bytes) of every P8_ handler, smallest first"
nm --size-sort -S -C "$B" | grep -o '[0-9a-f]* [tT] vmgen::handlers::P8_[A-Z_0-9]*' | awk '{ printf "%6d %s\n", strtonum("0x" $1), $3 }' | sort -n
