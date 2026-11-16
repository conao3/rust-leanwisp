# Leanwisp

Rust edition 2024, clap derive, serde, toml, flake-parts and treefmt-nix.
Follow shared notes/new-project-stack.md, notes/lang/rust.md and notes/lang/lean.md.
Run `nix develop --command make check` and `nix build` before committing.

Leanwisp is a canonical formatter: the layout depends only on the syntax and the
line width, never on the input's line breaks. Its rules come from Lean's
`doc/std/style.md` and the Mathlib style guidelines; where they allow
alternatives, README.md "Layout" records the chosen one. Only the per-command
fallbacks (`line-breaks`, then `source`) may follow the input's layout.

The Rust renderer owns width-sensitive layout, per-command validation and the
fallbacks. The Lean adapter loads parser and formatter extensions and returns
syntax fingerprints, token and command ranges, and formatting documents. It marks
tokens (declaration `:=`, attribute `]`, `where`, `first` alternatives) and
rewrites Lean's forced breaks into the documented layout: `attrline`,
`softblock`/`block` after `by` and `do`, and `command` markers that map
validation failures to commands.

Retain literal and comment contents. Validate syntax-tree equality, comments and
idempotence per command. Each layout rule has a golden test, and inputs that
differ only in line breaks must format identically. Acceptance on the Mathlib
sample is compilation, idempotence and no increase in Mathlib's style linter
warnings; the number of changed lines is reported, not a goal.

Mathlib regressions include comment placement and module docs, following-context
width checks, align measurement, semicolon proofs, pattern-let failure
continuations, original token spellings, `elab` syntax, untagged literal tokens
and notation referencing unelaborated local declarations. Keep the read-only
`tools/check-mathlib.py` sample and `docs/mathlib-evaluation.md` evidence
consistent. The optional Mathlib check requires existing Lean 4.35.0 build
artifacts; it does not belong in the default `make check`.
