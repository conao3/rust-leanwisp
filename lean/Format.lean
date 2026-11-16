import Lean
open Lean

-- Separate original token tags from synthetic tags already in parser trees.
def lexemeTagBase : Nat := 2 ^ 64
def continuationTagBase : Nat := 2 ^ 65

partial def fingerprint : Syntax → Json
  | .missing => toJson ("missing" : String)
  | .atom _ value => Json.arr #[toJson "atom", toJson value]
  | .ident _ raw value _ => Json.arr #[toJson "ident", toJson raw.toString, toJson value.toString]
  | .node _ kind args => Json.arr #[toJson kind.toString, Json.arr (args.map fingerprint)]

partial def document : Std.Format → Json
  | .nil => Json.arr #[toJson "nil"]
  | .line => Json.arr #[toJson "line"]
  | .align force => Json.arr #[toJson "align", toJson force]
  | .text value => Json.arr #[toJson "text", toJson value]
  | .nest indent child => Json.arr #[toJson "nest", toJson indent, document child]
  | .append a b => Json.arr #[toJson "append", document a, document b]
  | .group child behavior => Json.arr #[toJson "group", document child,
      toJson (behavior == .fill)]
  | .tag _ child => document child

partial def trivia : Syntax → Array Json
  | .node _ _ args => args.foldl (fun acc s => acc ++ trivia s) #[]
  | stx => match stx.getHeadInfo with
    | .original leading _ trailing _ =>
      #[toJson leading.toString, toJson trailing.toString]
    | _ => #[]

partial def markContinuation : Syntax → Syntax
  | .node info kind args =>
    if kind == `Lean.Parser.Term.doSeqItem then
      .node info kind (args.modify 0 fun stx => match stx with
        | .node _ k children =>
          let pos := stx.getPos?.getD 0
          .node (.synthetic ⟨pos.byteIdx + continuationTagBase⟩ pos) k children
        | _ => stx)
    else .node info kind (args.map markContinuation)
  | stx => stx

-- Keep trivia out of width/indentation groups. Rust restores comment-bearing gaps
-- at their corresponding token boundaries after laying out the syntax.
partial def withoutTrivia : Syntax → Syntax
  | .node info kind args =>
    let args := args.map withoutTrivia
    -- The continuation after a pattern-let failure branch is at the enclosing
    -- do indentation, rather than inside the failure branch.
    let args := if [ `Lean.Parser.Term.doLetElse, `Lean.Parser.Term.doLetExpr,
        `Lean.Parser.Term.doLetMetaExpr ].contains kind then
      args.modify (args.size - 1) markContinuation
      else args
    .node info kind args
  | .atom (.original _ pos _ tail) value =>
    .atom (if pos == tail then .original "".toRawSubstring pos "".toRawSubstring tail
      else .synthetic ⟨pos.byteIdx + lexemeTagBase⟩ tail) value
  | .ident (.original _ pos _ tail) raw name pre =>
    .ident (if pos == tail then .original "".toRawSubstring pos "".toRawSubstring tail
      else .synthetic ⟨pos.byteIdx + lexemeTagBase⟩ tail) raw name pre
  | stx => stx

partial def lexemes (source : String) : Syntax → Array (Nat × String)
  | .node _ kind args => if kind == choiceKind then lexemes source args[0]! else
    args.foldl (fun acc s => acc ++ lexemes source s) #[]
  | .atom (.original _ pos _ tail) _
  | .ident (.original _ pos _ tail) _ _ _ =>
    if pos == tail then #[] else
      #[(pos.byteIdx + lexemeTagBase, String.Pos.Raw.extract source pos tail)]
  | _ => #[]

partial def lexemeText (raw : String) : Std.Format → Std.Format
  | .text value =>
    if value.trimAscii.isEmpty || value.trimAscii.toString == raw.trimAscii.toString then
      .text value
    else
      let leading := String.ofList (value.toList.takeWhile (·.isWhitespace))
      let trailing := String.ofList ((value.toList.reverse.takeWhile (·.isWhitespace)).reverse)
      .text (leading ++ raw ++ trailing)
  | .append a b => .append (lexemeText raw a) (lexemeText raw b)
  | .nest n child => .nest n (lexemeText raw child)
  | .group child behavior => .group (lexemeText raw child) behavior
  | .tag tag child => .tag tag (lexemeText raw child)
  | fmt => fmt

partial def restoreLexemes (ids : Array (Nat × String)) : Std.Format → Std.Format
  | .tag tag child => if tag >= continuationTagBase then
      .nest (-4) (.align true ++ restoreLexemes ids child) else match ids.find? (fun entry => entry.1 == tag) with
    | some (_, raw) => lexemeText raw child
    | none => restoreLexemes ids child
  | .append a b => .append (restoreLexemes ids a) (restoreLexemes ids b)
  | .nest n child => .nest n (restoreLexemes ids child)
  | .group child behavior => .group (restoreLexemes ids child) behavior
  | fmt => fmt

partial def tokens : Syntax → Array Json
  | .node _ kind args => if kind == choiceKind then tokens args[0]! else args.foldl (fun acc s => acc ++ tokens s) #[]
  | stx => match stx.getRange? with
    | some r => if r.start == r.stop then #[] else
      #[Json.arr #[toJson r.start.byteIdx, toJson r.stop.byteIdx]]
    | none => #[]

partial def blocks : Syntax → Array Json
  | .node _ kind args =>
    let children := if kind == choiceKind then blocks args[0]! else
      args.foldl (fun acc s => acc ++ blocks s) #[]
    if kind == `Lean.Parser.Term.byTactic then
      match Syntax.getRange? (mkNode kind args) with
      | some r => children.push (Json.arr #[toJson r.start.byteIdx, toJson r.stop.byteIdx])
      | none => children
    else children
  | _ => #[]

partial def changesParser : Syntax → Bool
  | .node _ `Lean.Parser.Command.declaration _ => false
  | .node _ `Lean.Parser.Command.in _ => false
  | .node _ `Lean.Parser.Command.mutual _ => false
  | .atom _ value => ["syntax", "macro", "macro_rules", "notation", "infix", "infixl", "infixr",
      "prefix", "postfix", "declare_syntax_cat", "namespace", "section", "end", "open",
      "set_option"].contains value
  | .node _ _ args => args.any changesParser
  | _ => false

-- Inheriting documentation is irrelevant to parsing, and its referenced local
-- declaration has intentionally not been elaborated. Keep the source untouched.
def parserCommand (cmd : Syntax) : Syntax := Id.run do
  if cmd.isOfKind `Lean.Parser.Command.mixfix || cmd.isOfKind `Lean.Parser.Command.notation then
    let attrs := cmd[1][0][1].getArgs
    if attrs.size == 1 && (attrs[0]!.find? fun stx =>
        stx.isIdent && stx.getId == `inherit_doc).isSome then
      return cmd.setArg 1 (mkNullNode #[])
  return cmd

open Elab Frontend in
partial def collect : FrontendM (Array Syntax × Array Std.Format) := do
  let ctx ← getInputContext
  let cs ← getCommandState
  let ps ← getParserState
  let scope := cs.scopes.head!
  let (cmd, ps, messages) := Parser.parseCommand ctx
    { env := cs.env, options := scope.opts, currNamespace := scope.currNamespace,
      openDecls := scope.openDecls } ps cs.messages
  setParserState ps
  setMessages messages
  if messages.hasErrors then
    messages.forM fun msg => do IO.eprintln (← msg.toString)
    throw (IO.userError "invalid Lean syntax")
  let fmt ← runCommandElabM <| Command.liftCoreM <| PrettyPrinter.formatCategory `command (withoutTrivia cmd)
  let fmt := if cmd.isOfKind `Lean.Parser.Command.moduleDoc then
    Std.Format.text (String.Pos.Raw.extract ctx.inputString
      (cmd.getPos?.getD 0) (cmd.getTailPos?.getD 0)) else fmt
  if cmd.isOfKind `Lean.Parser.Command.eoi then return (#[cmd], #[fmt])
  if changesParser cmd then
    elabCommandAtFrontend (parserCommand cmd)
    if (← getCommandState).messages.hasErrors then
      (← getCommandState).messages.forM fun msg => do IO.eprintln (← msg.toString)
      throw (IO.userError "cannot elaborate parser-changing command")
  let (commands, docs) ← collect
  return (#[cmd] ++ commands, #[fmt] ++ docs)

unsafe def main : IO Unit := do
  initSearchPath (← findSysroot)
  enableInitializersExecution
  let source ← (← IO.getStdin).readToEnd
  let ctx := Parser.mkInputContext source "input.lean"
  let (header, ps, messages) ← Parser.parseHeader ctx
  if messages.hasErrors then throw (IO.userError "invalid module header")
  -- Match the compiler: module imports do not require private dependency artifacts.
  let level := if Elab.HeaderSyntax.isModule header then .exported else .private
  let env ← importModules (level := level) (loadExts := true) (Elab.headerToImports header) {} 0
  let (headerFmt, _) ← (PrettyPrinter.format Parser.Module.header.formatter (withoutTrivia header)).toIO
    { fileName := "input.lean", fileMap := ctx.fileMap } { env }
  let ((commands, docs), _) ← collect.run { inputCtx := ctx } |>.run
    { commandState := Elab.Command.mkState env {} (Options.setBool {} `quotPrecheck false), parserState := ps, cmdPos := ps.pos }
  let stx := mkNode `Lean.Parser.Module.module #[header, mkListNode commands]
  let fmt := headerFmt ++ Std.Format.joinSep docs.toList (Std.Format.line ++ Std.Format.line)
  IO.println (Json.mkObj [("Tree", fingerprint stx), ("Document", document (restoreLexemes (lexemes source stx) fmt)),
    ("Trivia", Json.arr (trivia stx)), ("Tokens", Json.arr (tokens stx)), ("Blocks", Json.arr (blocks stx))]).compress
