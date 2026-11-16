# Leanwisp

Rust edition 2024, clap derive, serde, toml, flake-parts and treefmt-nix.
Follow shared notes/new-project-stack.md, notes/lang/rust.md and notes/lang/lean.md.
Run `nix develop --command make check` and `nix build` before committing.

The Rust renderer owns width-sensitive layout, per-command validation and the
fallbacks (`line-breaks`, then `source`). The Lean adapter loads parser and
formatter extensions and returns syntax fingerprints, token and command ranges,
and formatting documents in which source line-break decisions are already
applied: marked tokens (declaration `:=`, attribute `]`, `where`, signature `:`,
binders) and token gaps decide whether a break is kept, joined or added by width.
Kept breaks carry the source's relative indentation (`break`); proof and `do`
blocks use `block`. Command markers map validation failures to commands.

Retain literal and comment contents. Validate syntax-tree equality, comments and
idempotence per command. Use golden fixtures for actual indentation, wrapping,
declaration spacing, imports and proof layouts: whitespace-only tests do not
establish formatter completeness. A change to layout rules must keep the
self-format test and the golden tests passing and must not raise the changed
line count of the Mathlib sample.

Mathlib regressions include comment placement and module docs, following-context
width checks, align measurement, semicolon proofs, pattern-let failure
continuations, original token spellings, `elab` syntax, untagged literal tokens
and notation referencing unelaborated local declarations. Keep the read-only
`tools/check-mathlib.py` sample and `docs/mathlib-evaluation.md` evidence
consistent. The optional Mathlib check requires existing Lean 4.35.0 build
artifacts; it does not belong in the default `make check`.
