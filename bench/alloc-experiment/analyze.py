"""Summarise results.txt from measure.sh: per candidate, cold/warm instructions
(callgrind) and cold/warm wall + peak RSS (median over rounds), each with its
ratio to the baseline computed within a round, then the median ratio.
usage: python3 -I analyze.py results.txt [baseline]"""
import sys
from collections import defaultdict
from statistics import median

path = sys.argv[1]
base = sys.argv[2] if len(sys.argv) > 2 else "glibc"
first_round = int(sys.argv[3]) if len(sys.argv) > 3 else 1  # earlier rounds = warm-up
# data[phase][workload][cand][round] = (val, rss, md5, check)
data = defaultdict(lambda: defaultdict(lambda: defaultdict(dict)))
for line in open(path):
    head, _, check = line.partition(" | ")
    f = head.split()
    if len(f) < 7 or not f[4].isdigit():
        continue
    phase, w, c, r, val, rss, md5 = f[:7]
    if int(r) < first_round:
        continue
    data[phase][w][c][int(r)] = (int(val), None if rss == "-" else int(rss), md5, check.strip())

cands = sorted({c for p in data.values() for w in p.values() for c in w},
               key=lambda c: (c != base, c))


def series(phase, cand, metric):
    """{round: value}; metric in cold, warm, lv (instructions or ns)."""
    d = data[phase]
    out = {}
    if metric == "lv":
        for r, v in d["lv"][cand].items():
            out[r] = v[0]
    else:
        for r, v1 in d["wp-r1"][cand].items():
            if metric == "cold":
                out[r] = v1[0]
            elif r in d["wp-r4"][cand]:
                out[r] = (d["wp-r4"][cand][r][0] - v1[0]) / 3
    return out


def cell(phase, cand, metric, scale, unit):
    s, b = series(phase, cand, metric), series(phase, base, metric)
    if not s:
        return "-"
    med = median(s.values()) / scale
    ratios = [s[r] / b[r] for r in s if r in b and b[r]]
    rt = f" ({median(ratios):.3f}x)" if cand != base and ratios else ""
    return f"{med:,.1f}{unit}{rt}"


def rss(cand, w):
    v = [x[1] for x in data["wall"][w][cand].values() if x[1]]
    if not v:
        return "-"
    b = [x[1] for x in data["wall"][w][base].values() if x[1]]
    rt = f" ({median(v) / median(b):.3f}x)" if cand != base and b else ""
    return f"{median(v) / 1024:,.0f} MB{rt}"


print(f"| candidate | WP cold instr (M) | WP warm instr/req (M) | WP cold wall | WP warm wall/req | WP peak RSS (r4) |"
      f" Laravel instr (M) | Laravel wall | Laravel peak RSS |")
print("|---|---|---|---|---|---|---|---|---|")
for c in cands:
    print(f"| {c} | {cell('cg', c, 'cold', 1e6, '')} | {cell('cg', c, 'warm', 1e6, '')} |"
          f" {cell('wall', c, 'cold', 1e6, ' ms')} | {cell('wall', c, 'warm', 1e6, ' ms')} | {rss(c, 'wp-r4')} |"
          f" {cell('cg', c, 'lv', 1e6, '')} | {cell('wall', c, 'lv', 1e6, ' ms')} | {rss(c, 'lv')} |")

print("\nrounds:", {p: sorted({r for w in data[p].values() for cd in w.values() for r in cd}) for p in data})
# Checks: within each round every candidate's output equals the baseline's.
bad = []
for phase, ws in data.items():
    for w, cs in ws.items():
        for c, rs in cs.items():
            for r, (_, _, md5, chk) in rs.items():
                ref = cs.get(base, {}).get(r)
                if ref and md5 != ref[2]:
                    bad.append((phase, w, c, r, md5, ref[2]))
print("output mismatches vs baseline in the same round:", bad or "none")
print("distinct WP r1 pages per round:", {r: len({cs[r][2] for cs in data['wall']['wp-r1'].values() if r in cs}) for r in sorted({r for cs in data['wall']['wp-r1'].values() for r in cs})})
print("laravel lines:", sorted({v[3] for p in data.values() for v in [x for cs in p['lv'].values() for x in cs.values()]}))
print("wall spread (min..max ms) cold WP:", {c: (round(min(series('wall', c, 'cold').values()) / 1e6), round(max(series('wall', c, 'cold').values()) / 1e6)) for c in cands if series('wall', c, 'cold')})
