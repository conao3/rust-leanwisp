# Mathlib sample evaluation

The sample is pinned to Mathlib `56a005855ea17e0b0958f73ec801ae140b0a1679`
and Lean 4.35.0: foundational modules plus the smallest and median-sized file
in ten areas, 31 files and 9,998 lines. It is a compatibility sample, not a run
across all 8,624 Lean files.

Leanwisp's layout is canonical, so formatting Mathlib changes lines that Mathlib
wrote in another style the guides allow. The acceptance criteria are therefore
compilation, idempotence and Mathlib's own style linters
(`-Dweak.linter.mathlibStandardSet=true`); the number of changed lines is reported
for review, not minimized.

## Reproduce

Build Mathlib's imported modules with Lean 4.35.0 first. The check only reads the
Mathlib source tree. Generated files, the isolated Lake project and logs go to
`--output-dir`, which must be outside Mathlib.

```sh
nix build
nix develop
# Run in the Mathlib project's Lake environment to inherit its built imports:
cd /path/to/mathlib
lake env python /path/to/rust-leanwisp/tools/check-mathlib.py \
  --mathlib-dir . \
  --binary /path/to/rust-leanwisp/result/bin/leanwisp \
  --output-dir /path/to/rust-leanwisp/.claude-dev/scratch/mathlib-check
```

An existing build cache can instead be supplied with `--lean-path`, a
colon-separated list of its `lib/lean` directories.

The script compiles each original with the style linters, formats it into a
separate tree with the default width of 100, compiles the result with the same
module name and linters, checks that a second pass changes nothing, counts
changed lines and kept commands, and verifies the original SHA-256 hashes. It
exits nonzero if any file fails, keeps a command unformatted or gains linter
warnings.

## Results (Leanwisp `428d9ab`)

| Check | Result |
|---|---:|
| Original compilation | 31/31 |
| Formatted without a file error | 31/31 |
| Recompilation of the formatted file | 31/31 |
| Idempotence | 31/31 |
| Mathlib style linter warnings, before → after | 0 → 0 |
| Lines over 100 columns, before → after | 3 → 3 (long comments) |
| Changed lines | 3,351 of 9,998 |
| Commands not formatted canonically | 10 in 7 files |
| Formatting time, all 31 files | 43.8 s (compiling the originals: 61.9 s) |
| Original checksums unchanged | 31/31 |

The changed lines are the canonical choices among layouts the guides allow:
headers and short bodies joined when they fit on one line, `t <;> u` and short
tactic blocks joined, `where` fields and attributes placed by the documented
rule, and two consecutive blank lines merged. See
[the Order/Basic diff](mathlib-order.diff) and [the Nat/Basic diff](mathlib-nat.diff).

Commands not formatted canonically:

- Lean has no formatter for the syntax, so the command keeps its source:
  `Data/List/Basic.lean:927` and `ModularForms/SlashInvariantForms.lean:93`
  (`term.pseudo.antiquot`), `CategoryTheory/Category/Basic.lean:204` (`«|»`).
- The canonical layout would not parse or would change the syntax tree, so the
  command keeps its source: `Algebra/Group/Basic.lean:1080`, `:1087`,
  `Analysis/SpecificLimits/Basic.lean:152`, `Order/Basic.lean:457`.
- The same, but keeping only the source line breaks (`line-breaks`):
  `Basic/Logic/Basic.lean:37`, `:70`, `ModularForms/SlashInvariantForms.lean:129`.

Per-file results, hashes, linter warnings and kept commands are in
[mathlib-results.json](mathlib-results.json). The check does not prove
equivalence of source-position-sensitive macros, and Mathlib beyond this sample
is untested.

## Speed comparison

The same 31 files were copied into one Lake project and checked on helios
(6 CPUs, load average 3–8 from other work, both tools run back to back) with Leanwisp `source check-source --paths Mathlib` and
[lean-fmt](https://github.com/jcreinhold/lean-fmt) `57c2390` built for Lean
4.35.0, `lean-fmt format --check <files>`, each with its default parallelism.
"Uncached" removes `~/.cache/leanwisp` and `.lean-fmt-cache` first; "cached" is
the immediately following second run.

| | Uncached | Cached |
|---|---:|---:|
| Leanwisp | 6.9 s | 0.09 s |
| lean-fmt | 33.3 s | 26.6 s |

Most of the uncached time is Lean loading each file's imports. Checking the 31
files one at a time takes 31.9 s in total, and checking only their headers (the
same imports, no commands) takes 28.3 s of it. A file without imports still
takes 0.27 s, mostly loading `Init`. For the heaviest header in the sample,
`NumberTheory/ModularForms/LevelOne.lean` (333,568 imported constants), Lean's
`importModules` spends about 0.5 s reading the `.olean` files, 0.4 s building
the constant tables and 0.5 s initializing the environment extensions. Parsing,
formatting and validating the 9,998 lines take the remaining 3.6 s.

A worker's Lean process reads each `.olean` file once for all of its files and
builds each file's environment from that data, applying Lean's import rules to
that file's own imports. With `--jobs 1` the sample takes 21.6–21.7 s, against
29.0–30.4 s when every file loads its imports in a fresh process (two
alternating runs each). With 6 workers each worker formats about five files, so
the gain is smaller: 6.8–7.0 s against 7.2–7.4 s.

A run traced with timestamps from both the Rust workers and the Lean processes
took 6.5 s with 6 workers. Of the 39.0 worker-seconds, building the file
environments took 22.8 s, Lean parsing, formatting and validation 11.1 s, the
Rust renderer and checks 2.7 s, starting the Lean processes 1.2 s, and workers
waiting for the last worker 1.1 s. Three traced runs under load average 4–7
gave the same shares within 2 percentage points.

Formatting one file at a time with one lean-fmt worker and no cache:

| File | Leanwisp | lean-fmt |
|---|---:|---:|
| `Data/Nat/Basic.lean` | 0.56 s | 0.79 s |
| `Order/Basic.lean` | 1.78 s | 4.79 s |
| `Algebra/Group/Basic.lean` | 2.25 s | 6.73 s |

Files that share imports are formatted in one Lean process, and `--paths` splits
a large group of them across workers when that shortens the run. The 31 sample
files all import different modules, so this does not apply to them. In a copy
where 29 of the files import the same 89 modules (the union of the sample's
imports), splitting the group across the 6 workers took 5.3–6.6 s, against
8.4–8.8 s with the whole group in one process (three alternating runs each, load average
about 4.7).
