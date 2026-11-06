# Allocator experiment (#319)

Which pure-Rust allocator php2 should use. Candidates: the system allocator
(glibc / macOS libmalloc), dlmalloc, rlsf, talc and rusty_alloc (a pure-Rust
mimalloc). php2 now uses rusty_alloc on every platform
(`crates/php-cli/src/main.rs`).

The candidates were built from commit 865cb19, where each was a php-cli
feature (`alloc-dlmalloc`, `alloc-rlsf`, `alloc-talc`, `alloc-rusty`; no
feature = the system allocator). To rerun the comparison, for example before
upgrading rusty_alloc, check that commit out, or add the candidate back as a
feature, and keep one binary per candidate as `php2-<name>`.

- `measure-linux.sh <cg|wall> <rounds>`: on [addis], inside
  `flock ~/p8-perf/.build.lock`. Callgrind instructions, or wall time and peak
  RSS, for WordPress cold (`--repeat 1`), warm (`--repeat 4`) and the Laravel
  Collection suite. The candidate order rotates each round, because the
  WordPress page's work drifts with the clock (#317). `wp/` must be a real
  directory, not a symlink: the script writes `../out`.
- `measure-mac.sh`: the same on [simien], wall time only (no valgrind).
- `analyze.py`: per-round medians and ratios.

## Results

`results-addis.txt` (Linux x86_64, glibc) and `results-simien-batch{1,2}.txt`
(macOS arm64). Medians, against the system allocator:

| | rusty_alloc, Linux | rusty_alloc, macOS | talc, Linux |
|---|---|---|---|
| WordPress cold, wall | −7.2% | −7% | −1.1% |
| WordPress warm per request, wall | −4.0% | −3% | +4.8% |
| Laravel suite, wall | −7.4% | −7% | +1.0% |
| WordPress cold / Laravel, instructions | −12.8% / −9.0% | n/a | −6.1% / −0.1% |
| peak RSS | +3–5% | about the same | +5% |

On macOS, dlmalloc and rlsf were 1–3% slower than the system allocator, and
ferroc did not compile. Every candidate produced byte-identical output.

rusty_alloc reads `MIMALLOC_*` / `RUSTY_ALLOC_*` once at startup, as glibc
reads `MALLOC_*` / `GLIBC_TUNABLES` and libmalloc reads `Malloc*`. Its only
build without that environment pass is single-threaded, for firmware.
