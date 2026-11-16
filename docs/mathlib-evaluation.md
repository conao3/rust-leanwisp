# Mathlib regression evaluation

The sample is pinned to Mathlib `56a005855ea17e0b0958f73ec801ae140b0a1679`
and Lean 4.35.0. It contains the same 31 files used for the initial evaluation:
foundational modules plus the smallest and median-sized file in ten areas.
This is a compatibility sample, not a run across all 8,624 Lean files.

## Changes driven by the sample

- Measure all-or-none and fill groups together with the following document.
  Reconsider flattening after hard lines, and handle alignment within a line.
  The grouping algorithm follows Lean's Apache-2.0
  [Format implementation](https://github.com/leanprover/lean4/blob/v4.35.0/src/Init/Data/Format/Basic.lean).
- Include `:= by` and `:= do` in width decisions. A wrapped semicolon proof
  starts its tactic sequence below `by`; parser ranges identify the proof block.
- Correct the indentation of continuations after pattern-let failure branches
  in `do`, including `let_expr`.
- Format syntax without comment trivia, then restore comment-bearing token gaps.
  Standalone comments retain their placement; only the next code token's
  indentation changes. Module documentation keeps its exact source text.
- Preserve original identifiers and operators, including `Prop.exists` and
  ASCII `>=`/`<=`, instead of changing their spellings during pretty-printing.
- Use the compiler's exported import level for `module` sources. Disable
  quotation prechecking for parser-only elaboration and omit a sole
  `inherit_doc` attribute from notation's parser setup, while preserving it in
  the formatted source. Ordinary declarations are still not elaborated.

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
colon-separated list of its `lib/lean` directories. The evaluation used existing
Mathlib, Batteries, Qq, Aesop, ProofWidgets, ImportGraph, LeanSearchClient and
Plausible build artifacts. It did not fetch or rebuild dependencies.

The script compiles each original, formats to a separate tree, compiles with the
same module name, checks a second pass and verifies original SHA-256 hashes.
It saves `manifest.json`, `results.json`, per-file logs and formatted sources.
It exits nonzero if any sample fails; unsupported inputs are not silently
counted as formatted successes. Missing imports are reported separately in the
per-file diagnostics.

## Results (2026-11-17)

| Check | Before (`3bd534b`) | Improved |
|---|---:|---:|
| Original compilation | 31/31 | 31/31 |
| Format, recompile and idempotence | 19/31 | 27/31 |
| Previous successful files still passing | — | 19/19 |
| Over-100-column lines in the common 19 successful files | 56 | 1 |
| Original checksums unchanged | 31/31 | 31/31 |

The one remaining long line in that common set is the original 120-character TODO
comment in Nat/Basic. Standalone comments and module-doc heading whitespace in
Nat/Basic now retain their source placement; see the
[actual Nat diff](mathlib-nat.diff). The maximum width metric counts
Unicode code points, including preserved comments.

Four inputs still fail safely:

- `Mathlib/Basic/Logic/Basic.lean`: `formatting would change the Lean syntax tree; input unchanged`.
- `Mathlib/Data/List/Basic.lean`: unknown formatter constant `term.pseudo.antiquot`.
- `Mathlib/CategoryTheory/Category/Basic.lean`: unknown formatter constant `«|»`.
- `Mathlib/NumberTheory/ModularForms/SlashInvariantForms.lean`: unknown formatter constant `term.pseudo.antiquot`.

Logic/Basic still exposes a changed `do` control-flow scope in the candidate. The
syntax-tree guard rejects it. The other three failures concern formatter support
for quotation/parser extensions. SlashInvariantForms previously stopped earlier
on a missing private dependency artifact; fixing imports revealed its formatter
error. These are not successful formats or environment-only exclusions.

Bulk application to all Mathlib is still unsupported. Review successful diffs:
some declarations become more vertical than handwritten Mathlib. The check does
not prove equivalence of source-position-sensitive macros, and passing this sample
does not establish compatibility with the untested files.

The complete portable per-file results and hashes are in
[mathlib-results.json](mathlib-results.json). Regular validation passed
`nix develop --command make check` (23 tests, format and clippy) and `nix build`.
`nix run . -- --output text source format-source --source "#eval (3: Nat)+4"`
produced `#eval (3 : Nat) + 4`.
