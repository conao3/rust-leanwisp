use leanwisp::{format_document, format_source};

fn stable(source: &str, width: usize) -> String {
    let result = format_source(source, width).unwrap();
    assert_eq!(format_source(&result, width).unwrap(), result);
    result
}

fn unchanged(source: &str) {
    assert_eq!(stable(source, 100), source);
}

#[test]
fn declaration_layout() {
    let result = stable(
        "import Std\ndef double(n:Nat):Nat:=n+n\n\n\n\ndef answer:Nat:=42\ndef next:Nat:=43\n",
        100,
    );
    assert_eq!(
        result,
        "import Std\ndef double (n : Nat) : Nat := n + n\n\ndef answer : Nat := 42\ndef next : Nat := 43\n"
    );
}

#[test]
fn proofs_do_match_structures() {
    for source in [
        "example (p q : Prop) (hp : p) (hq : q) : p ∧ q := by\n     constructor\n     · exact hp\n     · exact hq\n",
        "def action : IO Unit := do\n    let x := 1\n    IO.println x\n",
        "def f (x : Nat) : Nat :=\n match x with\n | 0 => 1\n | n+1 => n\n",
        "structure Person where\n    name : String\n    age : Nat\n",
        "namespace Foo\ndef x:Nat:=3\nend Foo\n",
    ] {
        stable(source, 80);
    }
}

#[test]
fn width_changes_layout() {
    let source = "def values : List Nat := [111111, 222222, 333333, 444444, 555555, 666666]\n";
    let wide = stable(source, 100);
    let narrow = stable(source, 35);
    assert_ne!(wide, narrow);
    assert!(narrow.lines().count() > wide.lines().count(), "{narrow}");
}

#[test]
fn comment_and_literal_contents() {
    for source in [
        "-- before\ndef x:Nat:=3 -- after\n-- eof\n",
        "/- outer /- inner -/ unchanged -/\ndef x : Nat := 3\n",
        "def s : String := \"not -- a comment : :=\"\n",
        "def s : String := r#\"raw : -- /- text\"#\n",
        "def c : Char := ':'\n",
        "/-- Documentation. -/\ndef «a:b» : Nat := 3\n",
        "def s : String := \"first\n  second\"\n",
        "def s := s!\"result {(3:Nat)}\"\n",
    ] {
        stable(source, 80);
    }
}

#[test]
fn imported_and_local_notation() {
    stable("import Lean\ndef x : Nat := 3\n", 100);
    let result = stable("infixl:65 \" +++ \" => Nat.add\n#check 1+++2\n", 80);
    assert!(result.contains("+++"));
}

#[test]
fn practice_and_large_fixture() {
    for source in [
        include_str!("fixtures/Basics.lean"),
        include_str!("fixtures/PracticeBasics.lean"),
        include_str!("fixtures/PracticeProofs.lean"),
    ] {
        stable(source, 80);
    }
}

#[test]
fn invalid_input_is_rejected() {
    for source in ["#eval (", "/- unclosed", "import Missing.Leanwisp.Module\n"] {
        assert!(format_source(source, 80).is_err());
    }
    assert!(format_source("#check Nat", 5).is_err());
}

#[test]
fn canonical_golden() {
    assert_eq!(
        stable("def double(n:Nat):Nat:=n+n\ndef answer:Nat:=\n  42", 100),
        "def double (n : Nat) : Nat := n + n\ndef answer : Nat := 42\n"
    );
}

#[test]
fn long_module() {
    let source: String = (0..150)
        .map(|i| format!("def number{i}:Nat:={i}\n"))
        .collect();
    let result = stable(&source, 80);
    assert_eq!(result.matches("def number").count(), 150);
}

#[test]
fn compile_formatted_fixtures() {
    let dir = std::env::temp_dir().join(format!("leanwisp-build-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, source) in [
        ("Basics", include_str!("fixtures/Basics.lean")),
        ("Proofs", include_str!("fixtures/PracticeProofs.lean")),
    ] {
        let file = dir.join(format!("{name}.lean"));
        std::fs::write(&file, stable(source, 80)).unwrap();
        let output = std::process::Command::new("lean")
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cli_and_lake_project() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("leanwisp-project-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("lakefile.toml"),
        "name = \"fixture\"\n[[lean_lib]]\nname = \"Support\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Support.lean"),
        "notation \"answer!\" => (42 : Nat)\n",
    )
    .unwrap();
    let build = Command::new("lake")
        .args(["build", "Support"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let file = dir.join("Example.lean");
    let source = "import Support\n#check answer!\ndef x:Nat:=3\n";
    std::fs::write(&file, source).unwrap();
    let binary = env!("CARGO_BIN_EXE_leanwisp");
    let check = Command::new(binary)
        .args(["source", "check-source", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(
        check.status.code(),
        Some(254),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), source);
    let write = Command::new(binary)
        .args(["source", "format-source", "--file"])
        .arg(&file)
        .arg("--write")
        .output()
        .unwrap();
    assert!(
        write.status.success(),
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );
    let check = Command::new(binary)
        .args(["source", "check-source", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let relative = Command::new(binary)
        .args(["source", "check-source", "--file", "Example.lean"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        relative.status.success(),
        "{}",
        String::from_utf8_lossy(&relative.stderr)
    );
    for args in [
        vec!["help"],
        vec!["source", "help"],
        vec!["source", "format-source", "--help"],
    ] {
        assert!(
            Command::new(binary)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    assert_eq!(
        Command::new(binary)
            .args(["source", "format-source"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(252)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn proof_and_comment_golden() {
    let source = "-- before\nexample (p q:Prop) (hp:p) (hq:q):p ∧ q:=by\n    constructor\n    · exact hp -- first\n    · exact hq\n-- after\n";
    let result = stable(source, 100);
    assert!(result.contains(":= by\n  constructor"), "{result}");
    for comment in ["-- before", "-- first", "-- after"] {
        assert_eq!(result.matches(comment).count(), 1, "{result}");
    }
    assert!(result.contains("  · exact hp"), "{result}");
}

#[test]
fn formatter_adapter_source() {
    let source = include_str!("../lean/Format.lean");
    let first = format_document(source, 100, None).unwrap();
    assert!(first.kept.iter().all(|k| !k.verbatim), "{:?}", first.kept);
    let second = format_document(&first.source, 100, None).unwrap();
    assert_eq!(second.source, first.source);
}

#[test]
fn exit_does_not_drop_following_source() {
    let result = stable(
        "#exit\n-- keep the following code\ndef later:Nat:=42\n",
        100,
    );
    assert!(result.contains("def later : Nat"), "{result}");
    assert!(result.contains("-- keep the following code"), "{result}");
}

#[test]
fn mathlib_comment_boundaries() {
    let source = "/-!\n# Basic operations\n\nModule documentation.\n-/\n\nstructure Entry where\n  value : Nat\n\n-- Shortcut instances\ndef entry:Entry:={value:=3}\n";
    let result = stable(source, 80);
    assert!(
        result.starts_with("/-!\n# Basic operations\n\nModule documentation.\n-/\n"),
        "{result}"
    );
    assert!(
        result.contains("\n\n-- Shortcut instances\ndef entry"),
        "{result}"
    );
}

#[test]
fn notation_can_reference_unelaborated_declaration() {
    stable(
        "namespace Local\ndef combine := Nat.add\ninfixl:65 \" +++ \" => combine\n#check 1+++2\nend Local\n",
        80,
    );
}

#[test]
fn groups_count_their_following_context() {
    use serde_json::json;
    let doc = json!([
        "append",
        [
            "group",
            [
                "append",
                ["text", "123456"],
                ["append", ["line"], ["text", "789"]]
            ],
            false
        ],
        ["text", " tail"]
    ]);
    assert_eq!(leanwisp::render(&doc, 12), "123456\n789 tail\n");
}

#[test]
fn wrapped_semicolon_proof_starts_below_by() {
    let source =
        "theorem repeated_true : True := by trivial; trivial; trivial; trivial; trivial; trivial\n";
    let result = stable(source, 40);
    assert!(result.contains(":= by\n  "), "{result}");
}

#[test]
fn preserve_identifier_spelling() {
    let result = stable("def Prop.exists : Nat := 3\n#check Prop.exists\n", 80);
    assert!(result.contains("Prop.exists"), "{result}");
    assert!(!result.contains("«Prop»"), "{result}");
}

#[test]
fn inherit_doc_on_local_notation() {
    let result = stable(
        "namespace Local\ndef combine := Nat.add\n@[inherit_doc] infixl:65 \" +++ \" => combine\n#check 1+++2\nend Local\n",
        80,
    );
    assert!(result.contains("@[inherit_doc]"), "{result}");
}

#[test]
fn do_pattern_let_continuation_is_outside_failure_branch() {
    let source = "def first (xs : List Nat) : Option Nat := do\n  let x :: _ := xs | return none\n  return some x\n";
    let result = stable(source, 80);
    assert!(result.contains("\n  return some x"), "{result}");
}

#[test]
fn by_introducer_counts_toward_line_width() {
    let source = "theorem some_very_long_theorem_name (a b : Nat) : a + b = b + a := by\n  exact Nat.add_comm a b\n";
    let result = stable(source, 60);
    assert!(
        result.lines().all(|line| line.chars().count() <= 60),
        "{result}"
    );
}

#[test]
fn preserve_ascii_operator_spelling() {
    let result = stable("#check (3>=2)\n#check (2<=3)\n", 80);
    assert!(result.contains("3 >= 2"), "{result}");
    assert!(result.contains("2 <= 3"), "{result}");
}

#[test]
fn layout_does_not_depend_on_source_line_breaks() {
    let canonical = "theorem bar (a b : Nat) : a + b = b + a := Nat.add_comm a b\n";
    for source in [
        "theorem bar (a b : Nat) :\n    a + b = b + a :=\n  Nat.add_comm a b\n",
        "theorem bar (a b : Nat) : a + b = b + a :=\n  Nat.add_comm\n    a b\n",
        canonical,
    ] {
        assert_eq!(stable(source, 100), canonical);
    }
    let proof = "example (p : Prop) (hp : p) : p ∧ p := by constructor <;> exact hp\n";
    assert_eq!(
        stable(
            "example (p : Prop) (hp : p) : p ∧ p := by\n  constructor <;>\n  exact hp\n",
            100
        ),
        proof
    );
    let wide = stable(
        "theorem foo (h : 1 < 2) :\n    1 < 2 := by\n      exact h\n",
        100,
    );
    assert_eq!(wide, "theorem foo (h : 1 < 2) : 1 < 2 := by exact h\n");
}

#[test]
fn style_guide_declarations() {
    assert_eq!(
        stable("@[simp] theorem two : 2 = 2 := by\n  skip\n  rfl\n", 100),
        "@[simp]\ntheorem two : 2 = 2 := by\n  skip\n  rfl\n"
    );
    assert_eq!(
        stable("@[simp]\ntheorem one : 1 = 1 := rfl\n", 100),
        "@[simp] theorem one : 1 = 1 := rfl\n"
    );
    let long = stable(
        "@[simp] theorem two (a b c d e f g h : Nat) (h1 : a + b + c + d = e + f + g + h) : a + b + c + d = e + f + g + h := h1\n",
        100,
    );
    assert_eq!(
        long,
        "@[simp]\ntheorem two (a b c d e f g h : Nat) (h1 : a + b + c + d = e + f + g + h) :\n    a + b + c + d = e + f + g + h := h1\n"
    );
    let proof = stable(
        "theorem some_very_long_theorem_name_here (a b : Nat) : a + b = b + a ∧ b + a = a + b ∧ True := by exact ⟨Nat.add_comm a b, Nat.add_comm b a, trivial⟩\n",
        100,
    );
    assert!(
        proof.ends_with(":= by\n  exact ⟨Nat.add_comm a b, Nat.add_comm b a, trivial⟩\n"),
        "{proof}"
    );
    assert_eq!(
        stable("instance : Inhabited Nat where default := 0\n", 100),
        "instance : Inhabited Nat where\n  default := 0\n"
    );
}

#[test]
fn style_guide_blocks() {
    unchanged("def g (n : Nat) : Nat := Id.run do\n  let mut x := n\n  x := x + 1\n  return x\n");
    assert_eq!(
        stable(
            "def g (n : Nat) : Nat :=\n  Id.run do\n      let x := n\n      return x\n",
            100
        ),
        "def g (n : Nat) : Nat := Id.run do\n  let x := n\n  return x\n"
    );
    unchanged(
        "def f (k : Nat) (args : Array Nat) : Array Nat :=\n  if k == 1 then args\n  else if k == 2 then\n    let (a, b) := (args[0]!, args[1]!)\n    #[a, b]\n  else args\n",
    );
    unchanged("inductive Color where\n  | red\n  | green\n  deriving Repr\n");
    unchanged("def empty : List Nat := {}\n");
    assert_eq!(
        stable(
            "example : True := by\n  first\n  | trivial\n  | simp\n",
            100
        ),
        "example : True := by first | trivial | simp\n"
    );
    unchanged("example (a : Nat) : (haveI := a; a) = a := rfl\n");
}

#[test]
fn unsupported_and_disabled_commands_keep_their_source() {
    let source = "import Lean\nsyntax \"twice! \" term : term\nmacro_rules\n  | `(twice! $x) => `($x  +  $x)\ndef a:Nat:=twice! 3\n-- leanwisp: off\ndef   keep  :  Nat:=1\n-- leanwisp: on\ndef b:Nat:=2\n";
    let result = format_document(source, 100, None).unwrap();
    assert!(result.source.contains("`($x  +  $x)"), "{}", result.source);
    assert!(
        result.source.contains("def a : Nat := twice! 3"),
        "{}",
        result.source
    );
    assert!(
        result.source.contains("def   keep  :  Nat:=1"),
        "{}",
        result.source
    );
    assert!(
        result.source.contains("def b : Nat := 2"),
        "{}",
        result.source
    );
    let lines: Vec<_> = result.kept.iter().map(|k| (k.line, k.verbatim)).collect();
    assert_eq!(lines, vec![(3, true), (7, true)], "{:?}", result.kept);
    assert_eq!(
        format_document(&result.source, 100, None).unwrap().source,
        result.source
    );
}

#[test]
fn cli_formats_paths_with_project_config() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("leanwisp-paths-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src/.hidden")).unwrap();
    std::fs::write(dir.join("leanwisp.toml"), "line-width = 40\n").unwrap();
    let long = "def values : List Nat := [111111, 222222, 333333, 444444]\n";
    std::fs::write(dir.join("src/A.lean"), long).unwrap();
    std::fs::write(dir.join("src/B.lean"), "def b : Nat := 2\n").unwrap();
    std::fs::write(dir.join("src/.hidden/C.lean"), "def   c:Nat:=3\n").unwrap();
    let binary = env!("CARGO_BIN_EXE_leanwisp");
    let check = Command::new(binary)
        .args(["source", "check-source", "--paths"])
        .arg(dir.join("src"))
        .output()
        .unwrap();
    assert_eq!(check.status.code(), Some(254));
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    let files = report["Files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{report}");
    assert_eq!(files[0]["Changed"], true, "{report}");
    assert_eq!(files[1]["Changed"], false, "{report}");
    let write = Command::new(binary)
        .args(["source", "format-source", "--write", "--paths"])
        .arg(dir.join("src"))
        .output()
        .unwrap();
    assert!(
        write.status.success(),
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );
    let formatted = std::fs::read_to_string(dir.join("src/A.lean")).unwrap();
    assert!(
        formatted.lines().all(|l| l.chars().count() <= 40),
        "{formatted}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("src/.hidden/C.lean")).unwrap(),
        "def   c:Nat:=3\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cli_reuses_cached_results() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("leanwisp-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Cached.lean");
    std::fs::write(&file, "def cached:Nat:=1\n").unwrap();
    let binary = env!("CARGO_BIN_EXE_leanwisp");
    let run = |args: &[&str]| {
        let output = Command::new(binary)
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .args(["--output", "text", "source", "format-source", "--file"])
            .arg(&file)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    let first = run(&[]);
    assert_eq!(first, "def cached : Nat := 1\n");
    assert_eq!(
        std::fs::read_dir(dir.join("cache/leanwisp"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(run(&[]), first);
    assert_eq!(run(&["--no-cache"]), first);
    std::fs::write(&file, "def cached:Nat:=2\n").unwrap();
    assert_eq!(run(&[]), "def cached : Nat := 2\n");
    std::fs::remove_dir_all(dir).unwrap();
}
