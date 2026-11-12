#!/usr/bin/env python3
"""Where one loop iteration's machine instructions are, address by address.

Usage: lines.py <binary> <low.cg> <high.cg> [--rules <rules.txt>]
           [--arms <file suffix>=<arms.txt>] [--bias <hex>]

The two files are prof.sh's callgrind outputs of one case at 2000 and
22000 iterations. For every instruction address, (cost(high) - cost(low))
/ 20000 is how many times one iteration executes it; startup and compile
cancel. Each address is given its inline chain by addr2line (innermost
first), so code inlined into the loop keeps its own function, file and
line, and the arm of the loop it was inlined into.

One line per address: address, executions per iteration, phase, arm,
the instruction, callgrind's function, the chain.

--arms: the interpreter loop is one function; which arm of it an address
belongs to is the line of the loop's source its chain ends at. arms.txt
is lines of `line<TAB>name` (where each arm starts); an address whose
outermost chain entry is in a file ending with <file suffix> gets the
name of the last arm starting at or before its line. Any other address
gets callgrind's function as its arm (code that was not inlined).

--rules: lines of `phase<TAB>regex`; the first whose regex matches
"arm | instruction | function | chain" names the address's phase. Then
the phases' sums are printed and, per phase, the sums by arm, innermost
function and line.
"""
import bisect
import collections
import re
import subprocess
import sys

ITER = 20000


def read(path):
    """address -> (self cost, function, object) from a callgrind file written with --dump-instr=yes --compress-pos=no."""
    cost = collections.defaultdict(int)
    where = {}
    names = {}

    def name(kind, text):
        m = re.match(r"\((\d+)\)(?: (.*))?$", text)
        if not m:
            return text
        key = (kind, m.group(1))
        if m.group(2) is not None:
            names[key] = m.group(2)
        return names.get(key, "?")

    fn = ob = None
    skip = False
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            if line.startswith("fn="):
                fn = name("fn", line[3:])
            elif line.startswith("ob="):
                ob = name("ob", line[3:])
            elif line.startswith(("cfn=", "cob=", "cfi=", "cfl=", "fl=", "fi=", "fe=")):
                kind = line.split("=", 1)[0]
                name({"cfn": "fn", "cob": "ob"}.get(kind, "fl"), line.split("=", 1)[1])
            elif line.startswith("calls="):
                skip = True  # the next line is the call's inclusive cost
            elif line.startswith("0x"):
                if skip:
                    skip = False
                    continue
                parts = line.split()
                addr = int(parts[0], 16)
                cost[addr] += int(parts[2])
                where[addr] = (fn, ob)
    return cost, where


def chains(binary, addrs, bias):
    """address -> inline chain, innermost first: [(function, file:line), ...]."""
    out = subprocess.run(
        ["addr2line", "-e", binary, "-f", "-i", "-C", "-a"] + [hex(a - bias) for a in addrs],
        capture_output=True, text=True, check=True,
    ).stdout.splitlines()
    result = {}
    cur = None
    i = 0
    while i < len(out):
        if out[i].startswith("0x"):
            cur = int(out[i], 16) + bias
            result[cur] = []
            i += 1
        else:
            func = re.sub(r"::h[0-9a-f]{16}$", "", out[i])
            loc = re.sub(r" \(discriminator \d+\)", "", out[i + 1])
            loc = re.sub(r"^.*/crates/", "", loc)
            loc = re.sub(r"^.*/(Zend|main|ext|sapi)/", r"\1/", loc)
            loc = re.sub(r"^.*/rustc-[^/]*/library/", "std:", loc)
            loc = re.sub(r"^.*/build/[^/]*/out/", "out/", loc)
            result[cur].append((func, loc))
            i += 2
    return result


def disassembly(binary, addrs, bias):
    """address -> the instruction's text (objdump over the ranges the addresses cluster in)."""
    text = {}
    want = set(a - bias for a in addrs)
    files = sorted(want)
    start = 0
    while start < len(files):
        end = start
        while end + 1 < len(files) and files[end + 1] - files[end] < 0x4000:
            end += 1
        out = subprocess.run(
            ["objdump", "-d", "--no-show-raw-insn", f"--start-address={files[start]:#x}", f"--stop-address={files[end] + 16:#x}", binary],
            capture_output=True, text=True, check=True,
        ).stdout
        for line in out.splitlines():
            m = re.match(r"\s*([0-9a-f]+):\s+(.*)$", line)
            if m and int(m.group(1), 16) in want:
                text[int(m.group(1), 16) + bias] = re.sub(r"\s+", " ", m.group(2)).strip()
        start = end + 1
    return text


LIBRARY = re.compile(r"^/rustc/|/library/|\.cargo/registry|^\?")


def site(chain):
    """An address's place in the program's own source: the innermost frame
    of its inline chain that is not the standard library's, as `function
    file:line`, and the line of the outermost frame it was inlined into."""
    own = [(f, l) for f, l in chain if not LIBRARY.search(l)]
    if not own:
        return "(no line)"

    def short(loc):
        return loc.rsplit("/", 1)[-1]

    fn, loc = own[0]
    fn = re.sub(r"<[^<>]*>", "", fn).rsplit("::", 1)[-1] or fn
    text = f"{fn} {short(loc)}"
    if len(own) > 1:
        text += f" (in {short(own[-1][1])})"
    return text


def main():
    binary, low, high = sys.argv[1:4]
    opts = dict(zip(sys.argv[4::2], sys.argv[5::2]))
    # --rules <file>[,<file>...]: read in order, the first match wins.
    rules = []
    for path in opts["--rules"].split(",") if "--rules" in opts else []:
        with open(path) as f:
            for line in f:
                line = line.rstrip("\n")
                if line and not line.startswith("#"):
                    phase, rx = line.split("\t", 1)
                    rules.append((phase, re.compile(rx)))
    # --arms <file suffix>=<arms file>[,<file suffix>=<arms file>...]
    arm_files = []
    if "--arms" in opts:
        for part in opts["--arms"].split(","):
            suffix, path = part.split("=", 1)
            starts = []
            with open(path) as f:
                for line in f:
                    n, label = line.rstrip("\n").split("\t", 1)
                    starts.append((int(n), label))
            starts.sort()
            arm_files.append((suffix, starts))

    macro = {}

    def arm_of(fn, chain):
        """The arm an address belongs to: the last arm starting at or before
        the line of the outermost frame of its inline chain. None when that
        frame has no line or is a macro's (the caller then takes the
        neighbour's arm, or failing that the macro's name)."""
        if not arm_files:
            return fn or "?"
        if chain:
            loc = chain[-1][1]
            m = re.match(r"(.*):(\d+)$", loc)
            if m:
                for suffix, starts in arm_files:
                    if m.group(1).endswith(suffix):
                        i = bisect.bisect_right(starts, (int(m.group(2)), "\uffff")) - 1
                        # Before the first arm or past the last (`(after ...)`):
                        # another function of the same file.
                        if i < 0 or starts[i][1].startswith("(after "):
                            return fn or "?"
                        # A macro's lines (`(name!)`) belong to whichever
                        # arm used it: the neighbour's arm, as for no line.
                        if starts[i][1].endswith("!)"):
                            macro[tuple(chain)] = starts[i][1]
                            return None
                        return starts[i][1]
                return fn or "?"
            if any(loc.split(":")[0].endswith(s) for s, _ in arm_files) or loc.endswith("?"):
                return None
        return fn or "?"
    lo, _ = read(low)
    hi, where = read(high)
    delta = {a: hi[a] - lo.get(a, 0) for a in hi if hi[a] - lo.get(a, 0) != 0}
    # The main executable's load bias under valgrind: its addresses are
    # the file's plus this (the first LOAD segment is mapped page-aligned).
    main_ob = collections.Counter(where[a][1] for a in delta).most_common(1)[0][0]
    bias = int(opts["--bias"], 16) if "--bias" in opts else load_bias(binary, lo, hi, where, main_ob)
    addrs = sorted(a for a in delta if where[a][1] == main_ob)
    other = sum(delta[a] for a in delta if where[a][1] != main_ob)
    ch = chains(binary, addrs, bias)
    asm = disassembly(binary, addrs, bias)
    total = sum(delta.values())
    print(f"# total {total / ITER:.2f} per iteration ({total} over {ITER}); outside {main_ob}: {other / ITER:.2f}; bias {bias:#x}")
    by_phase = collections.defaultdict(float)
    by_phase_line = collections.defaultdict(lambda: collections.defaultdict(float))
    # An instruction with no line of its own (a spill, a reload, a jump the
    # compiler placed) or on a macro's line takes the arm of the executed
    # instruction before it in the same function when that one is within
    # NEAR bytes, else of the one after it; marked with a trailing `~`.
    NEAR = 64
    arms = {a: arm_of(where[a][0], ch.get(a, [])) for a in addrs}
    for i, a in enumerate(addrs):
        if arms[a] is not None:
            continue
        found = None
        for j in range(i - 1, -1, -1):
            b = addrs[j]
            if a - b > NEAR or where[b][0] != where[a][0]:
                break
            if arms[b] is not None:
                found = arms[b]
                break
        if found is None:
            for j in range(i + 1, len(addrs)):
                b = addrs[j]
                if b - a > NEAR or where[b][0] != where[a][0]:
                    break
                if arms[b] is not None:
                    found = arms[b]
                    break
        arms[a] = (found.rstrip("~") + "~") if found else macro.get(tuple(ch.get(a, [])), where[a][0] or "(no line)")
    for a in addrs:
        n = delta[a] / ITER
        fn = where[a][0]
        chain = " < ".join(f"{f}@{l}" for f, l in ch.get(a, []))
        arm = arms[a]
        phase = ""
        if rules:
            text = f"{arm} | {asm.get(a, '?')} | {fn} | {chain} | {a - bias:#x}"
            phase = next((p for p, rx in rules if rx.search(text)), "UNCLASSIFIED")
            # `phase one / phase two`: code that runs once for each (the
            # frame head, after a call and after a return), shared evenly.
            parts = phase.split(" / ")
            for p in parts:
                by_phase[p] += n / len(parts)
                by_phase_line[p][f"{arm.rstrip('~')}: {site(ch.get(a, []))}"] += n / len(parts)
        print(f"{a - bias:#x}\t{n:.3f}\t{phase}\t{arm}\t{asm.get(a, '?')}\t{fn}\t{chain}")
    if rules:
        if other:
            by_phase["(other objects: libc)"] += other / ITER
        print("\n# phases")
        for phase, n in sorted(by_phase.items(), key=lambda kv: -kv[1]):
            print(f"{n:9.2f}  {phase}")
        print(f"{sum(by_phase.values()):9.2f}  total")
        print("\n# by phase, function and line")
        for phase, n in sorted(by_phase.items(), key=lambda kv: -kv[1]):
            print(f"\n## {phase}: {n:.2f}")
            for key, m in sorted(by_phase_line[phase].items(), key=lambda kv: -kv[1]):
                print(f"{m:9.2f}  {key}")


def load_bias(binary, lo, hi, where, main_ob):
    """Under valgrind a position-independent executable is mapped at a fixed
    base; found by matching a symbol's file address with the lowest executed
    address of the function callgrind names the same."""
    syms = {}
    out = subprocess.run(["nm", "-C", "--defined-only", binary], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and parts[1] in "tT":
            syms.setdefault(re.sub(r"::h[0-9a-f]{16}$", "", parts[2]), []).append(int(parts[0], 16))
    lowest = {}
    for a in hi:
        fn, ob = where[a]
        if ob == main_ob and fn:
            key = re.sub(r"::h[0-9a-f]{16}$", "", fn)
            key = re.sub(r"'\d+$", "", key)
            lowest[key] = min(lowest.get(key, a), a)
    votes = collections.Counter()
    for fn, a in lowest.items():
        for s in syms.get(fn, []) if len(syms.get(fn, [])) == 1 else []:
            if (a - s) % 0x1000 == 0:
                votes[a - s] += 1
    if not votes:
        return 0
    return votes.most_common(1)[0][0]


if __name__ == "__main__":
    main()
