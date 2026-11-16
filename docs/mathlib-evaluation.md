# Mathlib sample evaluation

The sample is pinned to Mathlib `56a005855ea17e0b0958f73ec801ae140b0a1679`
and Lean 4.35.0: foundational modules plus the smallest and median-sized file
in ten areas, 31 files and 9,998 lines. It is a compatibility sample, not a run
across all 8,624 Lean files. Mathlib's sources already follow its style, so the
number of lines Leanwisp changes measures how far its layout departs from
handwritten Lean.

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

The script compiles each original, formats it into a separate tree with the
default width of 100, compiles the result with the same module name, checks
that a second pass changes nothing, counts changed lines and kept commands, and
verifies the original SHA-256 hashes. It exits nonzero if any file fails or keeps
a command unformatted.

## Results (Leanwisp `0914ff8`)

| Check | Result |
|---|---:|
| Original compilation | 31/31 |
| Formatted without a file error | 31/31 |
| Recompilation of the formatted file | 31/31 |
| Idempotence | 31/31 |
| Changed lines | 447 of 9,998 (4.5%) |
| Lines over 100 columns, before → after | 3 → 3 |
| Commands kept as source | 6 in 5 files |
| Original checksums unchanged | 31/31 |

Kept commands:

- `Data/List/Basic.lean:927`, `ModularForms/SlashInvariantForms.lean:93`: Lean has
  no formatter for `term.pseudo.antiquot`.
- `CategoryTheory/Category/Basic.lean:204`: Lean has no formatter for `«|»`.
- `Analysis/SpecificLimits/Basic.lean:83`: the layout would change the syntax tree.
- `NumberTheory/Divisors.lean:748`, `:758`: the layout of a set literal with
  source line breaks does not parse.

The changed lines are mostly these normalizations:

- two consecutive blank lines become one;
- `¬ a` becomes `¬a`, and similar spacing around prefix operators;
- tactics nested under `classical` or a match arm `=>` move to two columns
  from their parent, as in the second hunk of [the Nat diff](mathlib-nat.diff);
- lines over the width that the source wrapped differently are rewrapped.

See [the Order/Basic diff](mathlib-order.diff) for a 1,090-line file. Per-file
results, hashes and kept commands are in [mathlib-results.json](mathlib-results.json).

Bulk application to all of Mathlib remains untested beyond this sample, and the
check does not prove equivalence of source-position-sensitive macros. Review the
diff before applying Leanwisp to a large codebase.
