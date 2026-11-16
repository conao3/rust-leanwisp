# Leanwisp

Rust edition 2024, clap derive, serde, flake-parts and treefmt-nix.
Follow shared notes/new-project-stack.md, notes/lang/rust.md and notes/lang/lean.md.
Run `nix develop --command make check` and `nix build` before committing.

The Rust renderer owns width-sensitive layout. The Lean adapter loads parser and
formatter extensions and returns syntax fingerprints and formatting documents.
Retain literal and comment contents. Validate syntax-tree equality and idempotence.
Use golden fixtures for actual indentation, wrapping, declaration spacing, imports,
and proof layouts: whitespace-only tests do not establish formatter completeness.

Mathlib regressions include comment placement and module docs, following-context
width checks, semicolon proofs, pattern-let failure continuations, original token
spellings and notation referencing unelaborated local declarations. Keep the
read-only `tools/check-mathlib.py` sample and `docs/mathlib-evaluation.md` evidence
consistent. The optional Mathlib check requires existing Lean 4.35.0 build artifacts;
it does not belong in the default `make check`.
