# Leanwisp

Rust edition 2024, clap derive, serde, toml, leanfran, flake-parts and treefmt-nix.
Follow shared notes/new-project-stack.md, notes/lang/rust.md and notes/lang/lean.md.
Run `nix develop --command make check` and `nix build` before committing.

Leanwisp is a canonical formatter: the layout depends only on the syntax and the
line width, never on the input's line breaks. Its rules come from Lean's
`doc/std/style.md` and the Mathlib style guidelines; where they allow
alternatives, README.md "Layout" records the chosen one. Only the per-command
fallbacks (`line-breaks`, then `source`) may follow the input's layout.

## Lean and Rust

Deterministic rules that do not depend on I/O are Lean definitions with theorems,
embedded with `leanfran::lean!` and generated into Rust. Each file holds one block
and the Rust functions that convert its arguments; `src/lib.rs` and `src/main.rs`
call them on the real path. Do not reimplement their logic in Rust.

- `src/fallback.rs`: the per-command fallback. `escalate` moves a command from the
  canonical layout to `line-breaks` and then to `source`; `syntaxVerdict`,
  `commentVerdict`, `stabilityVerdict` and `parseBlame` decide whether a result is
  accepted and which commands a failure is blamed on (`blame`, `commandAt`).
  Theorems: a command climbs one level per escalation and no other command moves
  (`escalateOne_level`, `escalateOne_other`), a file with `count` commands is
  escalated at most `2 * count` times (`escalate_total`, `total_le`), every blamed
  index names a command of the original (`blame_bound`, `commandAt_lt`), a failure
  falls on the whole file only when the command trees are equal or one side has no
  command (`blame_file`), and a verdict passes exactly when trees, token counts or
  comments are equal (`syntaxVerdict_none_iff`, `commentVerdict_none_iff`).
- `src/schedule.rs`: `schedule` splits the groups of files with equal imports
  across `--paths` workers. Theorems: every file is assigned exactly once
  (`schedule_perm`), a scheduled group never mixes files of different imports
  (`schedule_within`), and splitting never lengthens the estimated run
  (`refine_makespan`).
- `src/policy.rs`: the accepted line width and its default, the number of jobs and
  workers, when `--write` is allowed and applied, the status of a file and the exit
  status. Theorems: `jobs_bounds`, `workers_bounds`, `check_never_writes`,
  `exitCode_cases`, `check_passes_iff`, `format_passes_iff`, `status_error_iff`,
  `status_formatted_iff`.
- `src/cache.rs`: the fields of a cache key and their FNV-1a hash (`fields`,
  `key`). Theorem: the fields determine the build, the adapter, the width, the
  source and every import with its `.olean` stamp (`fields_injective`).

Rust owns the command line, configuration and source files, the cache directory,
the adapter process, grouping files by their imports, and the renderer. The Lean
adapter (`native/Format.lean`, a native Lean 4.35.0 program) loads parser and formatter extensions and returns
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

## Generation and verification

- The build script (`build.rs`) generates Rust for every `lean!` block. A block
  whose source changed needs the `leanfran` executable of the dev shell
  (`LEANFRAN_CLI`): it writes the Lean source to `lean/LeanRustGen/`, checks the
  theorems with `lake build` and writes the Rust to `.leanfran/cache/`. Commit
  `lean/` and `.leanfran/cache/` together with the block. `nix build` sets
  `LEANFRAN_OFFLINE=1` and uses the committed cache only.
- `nix develop --command make check`: `nix fmt -- --ci`, Clippy,
  `lake -d lean build --wfail` (the theorems; a `sorry` fails), the comparison of
  `.leanfran/cache/` and `lean/` with what the build generated, and `cargo test`.
- `nix build`: the package with the adapter; its check phase runs the same tests.
