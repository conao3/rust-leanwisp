# Leanwisp

A Lean 4 source formatter with a Rust layout engine. Leanwisp normalizes spacing,
indentation and line wrapping into one canonical layout.
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

The output depends only on the syntax and the line width, not on how the input
was broken into lines. The rules follow Lean's
[standard library style](https://github.com/leanprover/lean4/blob/master/doc/std/style.md)
and the [Mathlib style guidelines](https://mathlib.org/contribute/style.html).
Where both allow alternatives, Leanwisp picks one:

- Spacing around tokens, binders, operators and `:`/`:=` is normalized
  (`(3: Nat)+4` becomes `(3 : Nat) + 4`).
- Lines are filled up to `--line-width` (default 100). A function application
  wraps at argument boundaries, an infix operator stays at the end of the line,
  and continuation lines are indented.
- Declarations are flush-left. A header that does not fit continues on lines
  indented four columns, with `:` at the end of a line. A body that fits stays on
  the `:=` line; otherwise it starts on the next line, indented two columns.
- Attributes stay on the declaration line when the whole declaration fits on one
  line, and go on their own line otherwise.
- `by`, `do` and `Id.run do` end the line, and their block is indented two
  columns from the start of the declaration or statement. A tactic block that
  fits stays on one line (`:= by simp`, `first | a | b`, `t <;> u`).
- `instance ... where` keeps `where` on the declaration line and puts each field
  on its own line, indented two columns. Inductive constructors, match arms,
  bullets and `if`/`else if`/`else` chains use the indentation of the guides.
- Between commands, zero or one blank line is kept; two or more become one.
  Blank lines inside a declaration are removed. Files end with one newline.
- Comments, string and character literals, identifiers and operator spellings
  (`>=`, `Prop.exists`) are kept as written.

Every result is parsed again: the syntax tree, comment contents and order must
match the input, and a second pass must produce the same text.

## Commands Leanwisp cannot format

Validation is per command. When a command cannot be laid out safely, Leanwisp
first keeps its line breaks from the source and only normalizes spacing and
indentation within lines (`line-breaks`). If that still fails, or if Lean has no
formatter for the command's syntax, the command keeps its source text
(`source`). These fallbacks are the only cases in which the output follows the
input's line breaks. The rest of the file is formatted. Each such command is
listed:

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
files, skipping names starting with `.` (such as `.lake`) and symlinks. Files are
formatted in parallel, one per CPU unless `--jobs N` is given.

## Speed

Each `--paths` worker (`--jobs`, default: the number of CPUs) keeps one Lean
process for all of its files. The process reads each imported module's `.olean`
files once and builds every file's environment from that data with Lean's own
import rules, so a file sees exactly the modules it imports. A file's
environment serves every validation parse of that file and the following files
with the same imports. Files that share imports go to the same worker, and a
large group of them is split across workers whenever that shortens the run.

The process keeps the parse of each command, so a reparse after a fallback only
parses the text after the first change, and command layouts are reused across
files when the preceding syntax-changing commands are unchanged. Commands are
formatted in parallel, and parses whose layout is not needed skip formatting.
Comment restoration and the idempotence check align tokens instead of
reparsing. Results are
cached in `$XDG_CACHE_HOME/leanwisp` (default `~/.cache/leanwisp`), keyed by the
source, the line width, the Leanwisp build and the `.olean` files of the imported
modules, so an unchanged file is not parsed again. `--no-cache` disables the
cache; `--debug` prints parse and render times.

On the 31-file Mathlib sample in [docs/mathlib-evaluation.md](docs/mathlib-evaluation.md)
(6 CPUs, `check-source --paths`, both measured back to back on the same busy
machine):

| | Uncached | Cached |
|---|---:|---:|
| Leanwisp | 6.9 s | 0.09 s |
| [lean-fmt](https://github.com/jcreinhold/lean-fmt) `57c2390` (`format --check`) | 33.3 s | 26.6 s |

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
rendering and file handling. The rules that decide a result are Lean definitions
with proved properties, generated into Rust with
[leanfran](https://github.com/conao3/lean-leanfran): which validation failures
send a command to which fallback, the width and indentation rules, the assignment
of `--paths` files to workers, the cache key, and the limits and exit statuses of
the command line. AGENTS.md lists the definitions, their theorems and the measured
reasons for the code that stays in Rust. A bundled, natively compiled Lean adapter
(`native/Format.lean`) uses the real parser and
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
nix develop --command make check  # rustfmt/nixfmt/ruff, clippy, Lean theorems, generated code, all tests
nix build                       # packaged application and integration tests
nix run . -- --version
```

Enable the optional pre-commit hook with `git config core.hooksPath .githooks`.
`lean/` and `.leanfran/cache/` are generated from the `lean!` blocks under `src/`;
the dev shell regenerates them on `cargo build`, and `make check` fails when the
committed files differ.
Tests include golden layouts for each rule above, narrow and wide widths,
literal and comment preservation, per-command fallbacks and opt-out markers,
150 declarations, Lake-local notation, CLI write/check/`--paths` behavior,
`leanwisp.toml`, compilation of formatted Lean, and formatting the adapter's own
source.

Mathlib compatibility is tracked with a read-only, repeatable sample check in
[docs/mathlib-evaluation.md](docs/mathlib-evaluation.md). It records compilation,
idempotence, Mathlib's style linter warnings before and after formatting, changed
lines, kept commands and source hashes. The check also records formatting time; on the
sample, formatting takes less time than compiling.
`tools/check-mathlib.py --help` describes the check inputs.

Apache-2.0. Leanwisp uses the Lean parser/formatter APIs pinned by `flake.lock`.
