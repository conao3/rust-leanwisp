# Leanwisp

A Lean 4 source formatter with a Rust layout engine. Leanwisp normalizes spacing,
indentation and line wrapping while keeping the line structure an author chose.
It supports Lean 4.35.0, standard library and Lake dependency imports (Mathlib
included), and local notation, macro and elab declarations.

```sh
# Format a file, writing only after validation succeeds.
nix run github:conao3/rust-leanwisp -- source format-source --file Example.lean --write
# Check without writing (254 means formatting changes are needed).
nix run . -- source check-source --file Example.lean
# Format every .lean file under directories, four files at a time.
nix run . -- source format-source --paths src tests --write --jobs 4
# An editor filter: source on stdin, formatted source on stdout.
nix run . -- --output text source format-source --source file:///dev/stdin
# Choose the target line width (default 100, supported range 20–300).
nix run . -- source format-source --file Example.lean --line-width 80
```

For example:

```lean
-- Input
def double(n:Nat):Nat:=n+n
theorem double_eq (n:Nat):double n=n+n:=by
     unfold double
     rfl
-- Output
def double (n : Nat) : Nat := n + n
theorem double_eq (n : Nat) : double n = n + n := by
  unfold double
  rfl
```

## Layout

Leanwisp lays out each command with Lean's own formatter document and then applies
these rules:

- Spacing around tokens, binders, operators and `:`/`:=` is normalized
  (`(3: Nat)+4` becomes `(3 : Nat) + 4`).
- A line break the source has after `:=`, `=>`, `↦`, `<|`, `<;>`, `by` or `do`,
  after a signature's `:`, between binders, and inside wrapping argument lists is
  kept. Breaks Leanwisp adds are only those needed for `--line-width`.
- Code the source keeps on one line stays joined where Lean's formatter would
  force a break: a short body after `:=`, `@[simp] theorem`, `first | a | b`,
  `(haveI := h; e)`. `instance ... where` keeps `where` on the declaration line.
- Proof and `do` blocks are indented two columns from the start of their
  declaration or statement; inductive constructors, structure fields, match
  arms and bullets get canonical indentation. Lines kept from the source keep
  their indentation relative to the previous line (at most four columns deeper).
- Between commands, zero or one blank line is kept as written; two or more
  become one. Files end with one newline.
- Comments, string and character literals, identifiers and operator spellings
  (`>=`, `Prop.exists`) are kept as written.

Every result is parsed again: the syntax tree, comment contents and order must
match the input, and a second pass must produce the same text.

## Commands Leanwisp cannot format

Validation is per command. When a command cannot be laid out safely, Leanwisp
first keeps its line breaks from the source and only normalizes spacing and
indentation within lines (`line-breaks`). If that still fails, or if Lean has no
formatter for the command's syntax, the command keeps its source text
(`source`). The rest of the file is formatted. Each such command is listed:

```text
Example.lean:12: kept source: Unknown constant `term.pseudo.antiquot`
```

and in JSON as `Kept: [{"Line": 12, "Kept": "source", "Reason": "..."}]`.
`--debug` prints each validation failure that moved a command to a fallback.
Invalid Lean syntax and missing imports are errors for the whole file, and the
input is not written.

Commands between `-- leanwisp: off` and `-- leanwisp: on` lines keep their source
text.

## Projects and configuration

Lake projects are discovered upward from `--file` (or the current directory for
stdin). `--project-dir PATH` selects one explicitly. Build its imported modules
first with `lake build`; Leanwisp runs its syntax adapter in `lake env` so local
and dependency modules, including Mathlib, use the project's search path. The
project's Lean version must match 4.35.0.

A `leanwisp.toml` found upward from the input sets the project's line width;
`--line-width` overrides it:

```toml
line-width = 100
```

`--paths` takes files and directories. Directories are searched for `.lean`
files, skipping names starting with `.` (such as `.lake`) and symlinks.

## CLI

CLI inputs are named parameters. `--source` accepts literal text or `file://`
contents; `--file` reads a UTF-8 file; `--paths` takes a list. JSON output is the
default: `Changed`, `Source` and `Kept` for one input, and `Files` with `Path`,
`Changed`, `Kept` and `Error` for `--paths`. Global `--output text` emits the
source (or one `status kept path` line per file for `--paths`), and
`--output table` reports change status. Use `leanwisp help`,
`leanwisp source help`, and subcommand `--help`.

Exit codes: 0 success, 252 CLI syntax errors, 254 input/formatting/check errors
(including files that need formatting under `check-source`). Only `--write`
writes, and a file changed during formatting is not overwritten. Symlink inputs
are rejected.

## Implementation

The Rust implementation owns command parsing, project discovery, document
rendering, width and indentation policies, validation, fallbacks and file
handling. A bundled, natively compiled Lean adapter uses the real parser and
formatter extensions to produce syntax fingerprints, comment trivia, token
ranges and a formatting document marked with the source's line-break decisions.
Imported modules are loaded; commands that change the parser or scope are
elaborated so later commands can use their syntax. Ordinary declarations and
`#eval` commands are not elaborated.

Limitations: syntax extensions need a corresponding Lean formatter, otherwise the
command keeps its source text. A syntax tree comparison does not prove
elaborated theorem equivalence for macros that inspect source positions. Line
endings outside literals/comments are normalized to LF. Windows is not
supported; Linux is tested, and the Nix flake also declares aarch64-darwin.

## Development

```sh
nix develop --command make check  # rustfmt/nixfmt/ruff, clippy, all tests
nix build                       # packaged application and integration tests
nix run . -- --version
```

Enable the optional pre-commit hook with `git config core.hooksPath .githooks`.
Tests include golden layouts for each rule above, narrow and wide widths,
literal and comment preservation, per-command fallbacks and opt-out markers,
150 declarations, Lake-local notation, CLI write/check/`--paths` behavior,
`leanwisp.toml`, compilation of formatted Lean, and formatting the adapter's own
source.

Mathlib compatibility is tracked with a read-only, repeatable sample check in
[docs/mathlib-evaluation.md](docs/mathlib-evaluation.md). It records compilation,
idempotence, changed lines, kept commands and source hashes.
`tools/check-mathlib.py --help` describes the check inputs.

Apache-2.0. Leanwisp uses the Lean parser/formatter APIs pinned by `flake.lock`.
