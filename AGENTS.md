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
(`pub mod model`) and the Rust functions that convert its arguments; `src/lib.rs`
and `src/main.rs` call them on the real path. Do not reimplement their logic in
Rust. A rule belongs in Lean when it is evaluated once per run, file, command,
group or line break.

- `src/fallback.rs`: the per-command validation and fallback. `escalate` moves a
  command from the canonical layout to `line-breaks` and then to `source`;
  `syntaxVerdict`, `commentVerdict`, `stabilityVerdict` and `parseBlame` decide
  whether a result is accepted and which commands a failure is blamed on (`blame`,
  `commandAt`); `lineKept` selects the commands reported as `line-breaks`.
  Theorems: a command climbs one level per escalation and no other command moves
  (`escalateOne_level`, `escalateOne_other`), a file with `count` commands is
  escalated at most `2 * count` times (`escalate_total`, `total_le`), a failure
  blamed on distinct commands that are not yet kept verbatim always gives a next
  attempt (`escalate_isSome`), `lineKept` holds exactly the applied commands that are
  not verbatim (`mem_lineKept_iff`), every blamed index names a command of the
  original (`blame_bound`, `commandAt_lt`, `commandAt_last`, `stabilityVerdict_within`,
  `parseBlame_within`, stated with the predicate `Blame.within`), a failure falls on the whole file only when the command trees
  are equal or one side has no command (`blame_file`), a verdict passes exactly
  when trees and token counts, or comments, are equal (`syntaxVerdict_none_iff`,
  `commentVerdict_none_iff`), a comment failure is blamed on the first command whose
  comments differ, or on the file when the per-command comments agree
  (`commentVerdict_blame`), a stability failure is reported exactly when the two
  renderings differ (`stabilityVerdict_isSome`), and a parse failure falls on the
  file exactly when the command starts or the error offset are missing
  (`parseBlame_file_iff`).
- `src/layout.rs`: the width and indentation rules of the renderer. `fits` and
  `needsContext` decide whether a group stays on one line; `softblockIndent`,
  `blockIndent` and `breakIndent` give the indentation after `by`/`do` blocks and
  forced breaks; `alignAction` places an aligned continuation. Theorems: a block
  is indented at most two columns from the start of its statement and never past
  the nesting indentation, and its indentation is one of those bounds
  (`softblockIndent_le`, `softblockIndent_attained`, `blockIndent_le`,
  `blockIndent_attained`), a group is flat exactly when it has no forced break and
  fits the room (`fits_iff`), the width after a group is measured exactly when the
  group has no line break and fits the room (`needsContext_iff`), and an aligned
  continuation is indented at the start of a line, padded up to the alignment
  column before it, and wrapped at or past it (`alignAction_indent_iff`,
  `alignAction_pad`, `alignAction_wrap_iff`).
- `src/schedule.rs`: `schedule` splits the groups of files with equal imports
  across `--paths` workers. Theorems: every file is assigned exactly once
  (`schedule_perm`), a scheduled group never mixes files of different imports
  (`schedule_within`), and splitting never lengthens the estimated run
  (`refine_makespan`).
- `src/policy.rs`: the accepted line width and its default, the number of jobs and
  workers, which directory entries `--paths` formats (`entry`), how `--source` is
  read (`locate`), when `--write` is allowed and applied, the status of a file and
  the exit status. Theorems: `default_width_is_valid`, `jobs_bounds`,
  `jobs_requested`, `jobs_default`, `workers_bounds`,
  `hidden_and_symlinks_are_skipped`, `entry_format_iff`, `entry_descend_iff`,
  `locate_stdin`, `locate_literal`, `check_never_writes`, `writable_iff`,
  `exitCode_cases`, `check_passes_iff`, `format_passes_iff`, `status_error_iff`,
  `status_formatted_iff`, `status_changed_iff`.
- `src/cache.rs`: the fields of a cache key and their FNV-1a hash (`fields`,
  `key`). Theorem: the fields determine the build, the adapter, the width, the
  source and every import with its `.olean` stamp (`fields_injective`).

A theorem counts when it rules out a broken implementation. `audit/` is a Lake
package that runs [lean-assay](https://github.com/conao3/lean-assay) on the
generated functions Rust calls: `#assay` breaks each function in small ways and
runs the theorems about it on the broken versions. `lake -d audit build --wfail`
fails when a function has no theorem, when no broken version of it is refuted,
when a broken version satisfies every theorem but changes the output (a hole), or
when no input satisfies a theorem's hypotheses. It covers `entry`, `locate`,
`validWidth`, `lineWidth`, `jobs`, `workers`, `writable`, `writes`, `status`,
`exitCode`, `escalate`, `lineKept`, `syntaxVerdict`, `commentVerdict`,
`stabilityVerdict`, `parseBlame`, `importsFields`, `softblockIndent`,
`blockIndent`, `breakIndent`, `fits`, `needsContext`, `alignAction` and `schedule`:
155 broken versions, 140 refuted, 11 with the same output on every input tried,
4 that do not typecheck. A function added to a block gets a theorem and an
`#assay` line in the same change. `key` stays outside the audit: the theorems
about it are those of `fields` and `importsFields`, and `fields_injective` leaves
the byte count of the width field free (`#assay model.fields` reports the two
holes). The constants `usageStatus`, `failureStatus`, `minWidth` and `maxWidth` are
outside it too: a theorem about one of them alone would repeat its value.

Two Lean definitions specify code that runs in Rust. `measure` in `src/layout.rs`
is the width measurement of a document (theorems: `measure_append_assoc`,
`measure_nil_append`, `measure_group`, `measure_hard_line`, `measure_finish_line`),
and `validRanges` in `src/fallback.rs` is the acceptance of the adapter's token
ranges (`validRanges_sound`: in order, without overlap, inside the source). The
unit tests of `src/lib.rs` compare the Rust `measure` and `valid_ranges` with them
on generated inputs and on the documents of the fixtures.

Rust owns the command line, configuration and source files, the cache directory,
the adapter process, grouping files by their imports, and every walk over
document nodes, tokens or source text. These stay in Rust for the measured cost of
crossing into generated code per node or token (release build, `native/Format.lean`:
746 lines, 32,885 document nodes, 6,729 tokens, 71 commands):

| Rust code | Measured |
|---|---|
| `measure`, `measure_groups`, the walk of `regroup` | rendering with `measure` called in Lean per item takes 201 ms; in Rust the whole render takes 3.4 ms |
| `render_marked`, `attach_block_introducers` | handing the document to Lean once takes 3.8 ms (1.6 ms to build the tree, 2.2 ms to convert and release it), more than the whole Rust render |
| `valid_ranges` | 985 µs in Lean for 6,729 ranges on every parse; 6.7 µs in Rust |
| `comments`, `skip_trivia`, `token_positions`, `layout_blocks`, `restore_comments` | one string passed to a generated function and back costs 113 ns, 0.76 ms for one pass over the token gaps; `token_positions` takes 131 µs and `restore_comments` 183 µs in Rust |

`imported_modules` reads the imports of a file's header. The list selects the
`.olean` stamps of the cache key and the groups of `schedule`. It stays in Rust
(0.25 µs per file): its specification is Lean's header grammar, which the adapter
applies when it parses the file, and a Lean definition of the scanner would carry
no theorem about that grammar. `error_offset` converts the line and column of an
adapter error message into a byte offset of the rendered text. Everything the
renderer and the text passes produce is validated on every run: the adapter parses
the result again, and the verdicts of `src/fallback.rs` accept or reject it.

A rule called per group or per line break costs 6 to 19 ns per call (5,495 groups
and 3,426 line breaks in the file above). `syntaxVerdict` takes 28 µs,
`commentVerdict` 112 µs, and `key` 90 µs for a 35 KB source (82 µs for the same
hash in Rust). `schedule` takes 2.6 to 14 ms for 7,000 files.

The Lean adapter (`native/Format.lean`, a native Lean 4.35.0 program) loads parser
and formatter extensions and returns syntax fingerprints, token and command
ranges, and formatting documents. It marks tokens (declaration `:=`, attribute
`]`, `where`, `first` alternatives) and rewrites Lean's forced breaks into the
documented layout: `attrline`, `softblock`/`block` after `by` and `do`, and
`command` markers that map validation failures to commands.

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
  `lake -d lean build --wfail` (the theorems; a warning or a `sorry` fails),
  `lake -d audit build --wfail` (the audit of the theorems with lean-assay; Lake
  fetches lean-assay and plausible at the revisions of `audit/lake-manifest.json`),
  the comparison of `.leanfran/cache/` and `lean/` with what the build generated, and
  `cargo test` (the comparisons with the Lean definitions, 100,000-element inputs
  to the generated functions on a worker's stack, and the formatter tests of `tests/format.rs`, which run the
  adapter).
- `nix build`: the package with the adapter; its check phase runs the same tests.
  `result/bin/leanwisp source format-source --file <file>` exercises the installed
  binary.
- A change that must not alter the output is compared with the previous binary on
  Lean's own sources: format a copy of `$LEAN_SYSROOT/src/lean` directories with
  both binaries (`format-source --paths <copy> --write --no-cache`) at two widths
  and compare the files, the JSON reports and the standard error.
