import Lean
open Lean

-- Separate original token tags from synthetic tags already in parser trees.
def lexemeTagBase : Nat := 2 ^ 64
def continuationTagBase : Nat := 2 ^ 65
def markBase : Nat := 2 ^ 66
def markScale : Nat := 2 ^ 48
def markDeclEq : Nat := 1
def markAttrClose : Nat := 2
def markWhere : Nat := 3
def markAlternative : Nat := 4
def markStructWith : Nat := 5
def breakTagBase : Nat := 2 ^ 67
def blockBreakTag : Nat := 2 ^ 68
def commandTagBase : Nat := 2 ^ 69
def attrLineTag : Nat := 2 ^ 70
def softBlockTag : Nat := 2 ^ 70 + 1
def fieldColumnTag : Nat := 2 ^ 70 + 2

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
  | .tag tag child => if tag == attrLineTag then Json.arr #[toJson "attrline"]
      else if tag == softBlockTag then Json.arr #[toJson "softblock"]
      else if tag == fieldColumnTag then Json.arr #[toJson "fieldcolumn"]
      else if tag >= commandTagBase then Json.arr #[toJson "command", toJson (tag - commandTagBase)]
      else if tag >= blockBreakTag then Json.arr #[toJson "block", toJson ((tag - blockBreakTag : Int) - 64)]
      else if breakTagBase <= tag then Json.arr #[toJson "break", toJson ((tag - breakTagBase : Int) - 64)]
      else document child

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

def markAtom (code : Nat) : Syntax → Syntax
  | stx@(.atom (.synthetic pos tail) value) =>
    if pos.byteIdx < lexemeTagBase || pos.byteIdx >= continuationTagBase then stx else
      .atom (.synthetic ⟨markBase + code * markScale + (pos.byteIdx - lexemeTagBase)⟩ tail) value
  | stx => stx

def markLayout (kind : SyntaxNodeKind) (args : Array Syntax) : Array Syntax :=
  if kind == `Lean.Parser.Command.declValSimple || kind == `Lean.Parser.Command.whereStructInst then
    args.modify 0 (markAtom (if kind == `Lean.Parser.Command.whereStructInst then markWhere else markDeclEq))
  else if kind == `Lean.Parser.Term.attributes && args.size == 3 then
    args.modify 2 (markAtom markAttrClose)
  else if kind == `Lean.Parser.Tactic.first && args.size == 2 then
    args.modify 1 fun alts => alts.setArgs (alts.getArgs.map fun alt =>
      if alt.getNumArgs > 0 then alt.setArg 0 (markAtom markAlternative alt[0]) else alt)
  else if kind == `Lean.Parser.Term.structInst && args.size > 1 && args[1]!.getNumArgs == 2 then
    args.modify 1 fun source => source.setArg 1 (markAtom markStructWith source[1])
  else args

-- Keep trivia out of width/indentation groups. Rust restores comment-bearing gaps
-- at their corresponding token boundaries after laying out the syntax.
partial def withoutTrivia : Syntax → Syntax
  | .node info kind args =>
    let args := markLayout kind (args.map withoutTrivia)
    -- The continuation after a pattern-let failure branch is at the enclosing
    -- do indentation, rather than inside the failure branch.
    let args := if [ `Lean.Parser.Term.doLetElse, `Lean.Parser.Term.doLetExpr,
        `Lean.Parser.Term.doLetMetaExpr ].contains kind then
      args.modify (args.size - 1) markContinuation
      else args
    let info := match args[0]? with
      | some (Syntax.atom (.synthetic pos _) _) =>
        if args.size == 1 && [numLitKind, strLitKind, charLitKind, scientificLitKind,
            nameLitKind].contains kind then .synthetic pos pos else info
      | some (Syntax.node (.synthetic pos _) child _) =>
        if args.size == 1 && [numLitKind, strLitKind, charLitKind, scientificLitKind,
            nameLitKind].contains child then .synthetic pos pos else info
      | _ => info
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
    if value == "\n|" || value == "\n| " then .text "\n" ++ .text (value.drop 1).toString else
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

partial def restoreLexemes (ids : Std.HashMap Nat String) : Std.Format → Std.Format
  | .tag tag child => if breakTagBase <= tag then .tag tag child
    else if continuationTagBase <= tag then
      .nest (-4) (.align true ++ restoreLexemes ids child) else match ids.get? tag with
    | some raw => lexemeText raw child
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
  | .atom _ value => ["syntax", "macro", "macro_rules", "elab", "elab_rules", "notation", "infix",
      "infixl", "infixr", "prefix", "postfix", "declare_syntax_cat", "namespace", "section", "end", "open",
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

partial def edgeText (last : Bool) : Std.Format → Option String
  | .text value => if value.trimAscii.isEmpty then none else some value.trimAscii.toString
  | .append a b =>
    let (x, y) := if last then (b, a) else (a, b)
    (edgeText last x).orElse fun _ => edgeText last y
  | .nest _ child | .group child _ | .tag _ child => edgeText last child
  | _ => none

partial def spaceLastLine : Std.Format → Std.Format
  | .append a .line => .append a (.text " ")
  | .append a b => .append a (spaceLastLine b)
  | .tag tag child => .tag tag (spaceLastLine child)
  | fmt => fmt

partial def flattenElseIf : Std.Format → Std.Format
  | .append a (.group (.nest n inner) behavior) =>
    let (a, inner) := (flattenElseIf a, flattenElseIf inner)
    if edgeText true a == some "else" && edgeText false inner == some "if" then
      .append (spaceLastLine a) (.group inner behavior)
    else .append a (.group (.nest n inner) behavior)
  | .append a b => .append (flattenElseIf a) (flattenElseIf b)
  | .nest n child => .nest n (flattenElseIf child)
  | .group child behavior => .group (flattenElseIf child) behavior
  | .tag tag child => .tag tag (flattenElseIf child)
  | fmt => fmt

inductive LayoutEvent where
  | token (code pos : Nat)
  | leaf (fmt : Std.Format) (fill : Bool)

partial def layoutEvents (fill : Bool) : Std.Format → Array LayoutEvent → Array LayoutEvent
  | .tag tag child, acc =>
    let acc := if tag >= markBase then
        acc.push (.token ((tag - markBase) / markScale) ((tag - markBase) % markScale))
      else if lexemeTagBase <= tag && tag < continuationTagBase then
        acc.push (.token 0 (tag - lexemeTagBase))
      else acc
    layoutEvents fill child acc
  | .append a b, acc => layoutEvents fill b (layoutEvents fill a acc)
  | .nest _ child, acc => layoutEvents fill child acc
  | .group child behavior, acc => layoutEvents (behavior == .fill) child acc
  | fmt, acc => acc.push (.leaf fmt fill)

def isBlank : Std.Format → Bool
  | .nil => true
  | .text value => value.isEmpty
  | _ => false

def isHardBreak : Std.Format → Bool
  | .text value => value.contains '\n' && value.all Char.isWhitespace
  | _ => false

def isSpace : Std.Format → Bool
  | .line => true
  | .text value => !value.isEmpty && value.all Char.isWhitespace
  | _ => false

def lineIndent (bytes : ByteArray) (pos : Nat) : Int := Id.run do
  let mut start := pos
  while start > 0 && bytes.get! (start - 1) != 10 do start := start - 1
  let mut stop := start
  while stop < bytes.size && bytes.get! stop == 32 do stop := stop + 1
  return stop - start

def tacticKeywords : List String :=
  ["·", "exact", "refine", "refine'", "apply", "use", "exists", "show", "calc", "obtain", "rcases",
    "rintro", "intro", "simpa", "convert", "trans", "change", "specialize", "unfold"]

structure Token where
  code : Nat
  pos : Nat
  leaf : Nat
  deriving Inhabited

def layoutReplacements (source : String) (lengths : Std.HashMap Nat Nat) (conservative : Bool)
    (fmt : Std.Format) :
    Std.HashMap Nat Std.Format := Id.run do
  let bytes := source.toUTF8
  let mut leaves : Array Std.Format := #[]
  let mut fills : Array Bool := #[]
  let mut tokens : Array Token := #[]
  for event in layoutEvents false fmt #[] do
    match event with
    | .token code pos => tokens := tokens.push { code, pos, leaf := leaves.size }
    | .leaf leaf fill =>
      leaves := leaves.push leaf
      fills := fills.push fill
  let text (t : Token) : String :=
    String.Pos.Raw.extract source ⟨t.pos⟩ ⟨t.pos + lengths.getD t.pos 0⟩
  let breakBetween (a b : Token) : Bool :=
    let stop := a.pos + lengths.getD a.pos 0
    stop <= b.pos && (String.Pos.Raw.extract source ⟨stop⟩ ⟨b.pos⟩).contains '\n'
  let next (i : Nat) : Option Nat := Id.run do
    let mut j := i + 1
    while j < leaves.size && isBlank leaves[j]! do j := j + 1
    return if j < leaves.size then some j else none
  let prev (i : Nat) : Option Nat := Id.run do
    let mut j := i
    while j > 0 && isBlank leaves[j - 1]! do j := j - 1
    return if j > 0 then some (j - 1) else none
  let mut result : Std.HashMap Nat Std.Format := {}
  let mut decided : Std.HashSet Nat := {}
  let breakAt (before after : Token) : Std.Format :=
    let delta := lineIndent bytes after.pos - lineIndent bytes before.pos
    .tag (breakTagBase + (64 + min 4 (max (-64) delta)).toNat) (.text "\n")
  let blockAt (before after : Token) : Std.Format :=
    let delta := lineIndent bytes after.pos - lineIndent bytes before.pos
    .tag (blockBreakTag + (64 + min 4 (max (-64) delta)).toNat) (.text "\n")
  for i in [1:tokens.size] do
    let (before, after) := (tokens[i - 1]!, tokens[i]!)
    if text before == "{" && text after == "}" && before.pos + 1 == after.pos then
      for j in [before.leaf:after.leaf] do
        if isSpace leaves[j]! then
          result := result.insert j .nil
          decided := decided.insert j
      if leaves[after.leaf]! matches .text " }" then result := result.insert after.leaf (.text "}")
  if conservative then
    for i in [1:tokens.size] do
      let (before, after) := (tokens[i - 1]!, tokens[i]!)
      let range := (List.range (after.leaf - before.leaf)).map (before.leaf + ·)
      let breaks := range.filter fun j => leaves[j]! matches .line || isHardBreak leaves[j]!
      let aligns := range.filter fun j => leaves[j]! matches .align _
      let chosen := if !breakBetween before after then breaks.head?
        else if (text before == "by" || text before == "do") && !aligns.isEmpty then aligns.head?
        else breaks.getLast?
      for j in range do
        if leaves[j]! matches .align _ && some j != chosen then result := result.insert j .nil
      let mut spaced := false
      for j in range do
        if breaks.contains j || some j == chosen then
          result := result.insert j (if breakBetween before after then
              (if some j != chosen then .text " "
                else if text before == "by" || text before == "do" then blockAt before after
                else breakAt before after)
            else if spaced then .nil else .text " ")
          spaced := true
        else if !(isSpace leaves[j]! || isBlank leaves[j]!) then spaced := false
    return result
  for i in [0:tokens.size] do
    let token := tokens[i]!
    if token.code == markDeclEq then
      if let some j := next token.leaf then
        if isHardBreak leaves[j]! then result := result.insert j .line
    else if token.code == markAttrClose then
      if let some j := next token.leaf then
        if isHardBreak leaves[j]! then result := result.insert j (.tag attrLineTag .nil)
        else if isSpace leaves[j]! then result := result.insert j (.nest (-2) (.tag attrLineTag .nil))
    else if token.code == markWhere then
      if let some j := prev token.leaf then
        if leaves[j]! matches .line then result := result.insert j (.text " ")
      if let some j := next token.leaf then
        if leaves[j]! matches .line then result := result.insert j (.text "\n")
    else if token.code == markAlternative then
      if let some j := prev token.leaf then
        if isHardBreak leaves[j]! then result := result.insert j .line
    else if token.code == markStructWith then
      if let some j := next token.leaf then
        if isSpace leaves[j]! then result := result.insert j (leaves[j]! ++ .tag fieldColumnTag .nil)
  for i in [1:tokens.size] do
    let (before, after) := (tokens[i - 1]!, tokens[i]!)
    for j in [before.leaf:after.leaf] do
      if result.contains j then continue
      if text before == "by" || text before == "do" then
        if leaves[j]! matches .align _ || isHardBreak leaves[j]! then
          result := result.insert j (.tag (blockBreakTag + 66) (.text "\n"))
        else if leaves[j]! matches .line then
          result := result.insert j (.tag softBlockTag .nil)
      else if text before == ";" && isHardBreak leaves[j]! then
        result := result.insert j .line
      else if text before == "{" && leaves[j]! matches .align _ then
        result := result.insert j (.text " ")
      else if tacticKeywords.contains (text before) && leaves[j]! matches .line then
        result := result.insert j (.text " ")
  return result

partial def applyReplacements (result : Std.HashMap Nat Std.Format) :
    Std.Format → StateM Nat Std.Format
  | .tag tag child => do
    let child ← applyReplacements result child
    return if tag >= markBase then .tag (lexemeTagBase + (tag - markBase) % markScale) child
      else .tag tag child
  | .append a b => return .append (← applyReplacements result a) (← applyReplacements result b)
  | .nest indent child => return .nest indent (← applyReplacements result child)
  | .group child behavior => return .group (← applyReplacements result child) behavior
  | fmt => do
    let i ← get
    set (i + 1)
    return result.getD i fmt

def applyLayoutMarks (source : String) (lengths : Std.HashMap Nat Nat) (conservative : Bool)
    (fmt : Std.Format) : Std.Format :=
  (applyReplacements (layoutReplacements source lengths conservative fmt) fmt).run' 0

partial def dropLastBreak : Std.Format → Std.Format
  | .append a (.text "\n") => a
  | .append a b => .append a (dropLastBreak b)
  | .group child behavior => .group (dropLastBreak child) behavior
  | fmt => fmt

def blankLineBetween (gap : String) : Bool :=
  let lines := gap.splitOn "\n"
  lines.length > 2 && (lines.drop 1 |>.dropLast).any (·.all Char.isWhitespace)

def offRegions (source : String) : Array (Nat × Nat) := Id.run do
  let mut regions := #[]
  let mut start : Option Nat := none
  let mut pos := 0
  for line in source.splitOn "\n" do
    let text := line.trimAscii.toString
    if text == "-- leanwisp: off" && start.isNone then start := some pos
    if text == "-- leanwisp: on" then
      if let some s := start then regions := regions.push (s, pos)
      start := none
    pos := pos + line.utf8ByteSize + 1
  if let some s := start then regions := regions.push (s, source.utf8ByteSize + 1)
  return regions

structure Entry where
  command : Syntax
  env : Environment
  scope : Elab.Command.Scope
  before : Elab.Frontend.State
  stop : Nat
  tree : UInt64
  tokens : Array Json
  trivia : Array Json
  blocks : Array Json
  lexemes : Array (Nat × String)
  prefixHash : UInt64
  envHash : UInt64

def commentTrivia (stx : Syntax) : Array Json :=
  (trivia stx).filter fun
    | .str value => value.any (· == '-')
    | _ => false

open Elab Frontend in
partial def collect (seed : UInt64) (acc : Array Entry) : FrontendM (Array Entry) := do
  let before ← get
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
    let texts ← messages.toList.mapM (·.toString)
    throw (IO.userError (String.intercalate "\n" texts ++ "\ninvalid Lean syntax"))
  let previous := acc.back?.map (·.stop) |>.getD before.parserState.pos.byteIdx
  let prefixHash := mixHash (acc.back?.map (·.prefixHash) |>.getD seed)
    (hash (String.Pos.Raw.extract ctx.inputString ⟨previous⟩ ps.pos))
  let envHash := match acc.back? with
    | some last => if changesParser last.command then
        mixHash last.envHash (hash (String.Pos.Raw.extract ctx.inputString
          (last.command.getPos?.getD 0) (last.command.getTailPos?.getD 0)))
      else last.envHash
    | none => seed
  let acc := acc.push {
    command := cmd, env := cs.env, scope, before, stop := ps.pos.byteIdx,
    prefixHash, envHash,
    tree := hash (fingerprint cmd).compress, tokens := tokens cmd, trivia := commentTrivia cmd,
    blocks := blocks cmd, lexemes := lexemes ctx.inputString cmd }
  if cmd.isOfKind `Lean.Parser.Command.eoi then return acc
  if changesParser cmd then
    elabCommandAtFrontend (parserCommand cmd)
    modify fun st => { st with commandState := { st.commandState with messages := {} } }
  collect seed acc

def formatCommand (ctx : Parser.InputContext) (entry : Entry) : IO (Except String Std.Format) := do
  let core : Core.Context := {
    fileName := ctx.fileName, fileMap := ctx.fileMap, options := entry.scope.opts,
    currNamespace := entry.scope.currNamespace, openDecls := entry.scope.openDecls }
  try
    let (fmt, _) ← (PrettyPrinter.formatCategory `command (withoutTrivia entry.command)).toIO
      core { env := entry.env }
    return .ok (flattenElseIf fmt)
  catch ex => return .error ex.toString

structure Memo where
  formats : Std.HashMap String (Except String Std.Format) := {}
  documents : Std.HashMap String Json := {}

def commandDocuments (memo : IO.Ref Memo) (ctx : Parser.InputContext) (verbatim lines : Array Nat)
    (off : Array (Nat × Nat)) (entries : Array Entry) : IO (Array Json × Array Json) := do
  let source := ctx.inputString
  let mut jobs : Array (String × Std.Format × Option (Task (Except IO.Error (Except String Std.Format)))) := #[]
  for entry in entries, i in [0:entries.size] do
    let cmd := entry.command
    let start := (cmd.getPos?.getD 0).byteIdx
    let text := String.Pos.Raw.extract source (cmd.getPos?.getD 0) (cmd.getTailPos?.getD 0)
    let original := Std.Format.text text
    let key := s!"{start}:{entry.prefixHash}:{text.utf8ByteSize}"
    let documentKey := s!"{lines.contains i}:{entry.envHash}:{text}"
    if cmd.isOfKind `Lean.Parser.Command.moduleDoc then jobs := jobs.push ("", original, none)
    else if verbatim.contains i then jobs := jobs.push ("requested", original, none)
    else if off.any (fun (a, b) => a <= start && start < b) then jobs := jobs.push ("disabled", original, none)
    else if (← memo.get).documents.contains documentKey then
      jobs := jobs.push (key, original, some (.pure (.ok (.ok .nil))))
    else if let some result := (← memo.get).formats.get? key then
      jobs := jobs.push (key, original, some (.pure (.ok result)))
    else
      jobs := jobs.push (key, original, some (← IO.asTask (formatCommand ctx entry)))
  let mut docs := #[]
  let mut kept := #[]
  for (key, original, job) in jobs, entry in entries, i in [0:jobs.size] do
    let start := (entry.command.getPos?.getD 0).byteIdx
    let keepIt (reason : String) := Json.arr #[toJson i, toJson start, toJson reason]
    let text := String.Pos.Raw.extract source (entry.command.getPos?.getD 0) (entry.command.getTailPos?.getD 0)
    let documentKey := s!"{lines.contains i}:{entry.envHash}:{text}"
    match job with
    | none =>
      docs := docs.push (document (.text original.pretty))
      if !key.isEmpty then kept := kept.push (keepIt key)
    | some task =>
      if let some doc := (← memo.get).documents.get? documentKey then
        docs := docs.push doc
        continue
      let result ← IO.ofExcept (← IO.wait task)
      memo.modify fun m => { m with formats := m.formats.insert key result }
      match result with
      | .ok fmt =>
        let lengths := entry.lexemes.foldl (init := {}) fun acc (tag, raw) =>
          acc.insert (tag - lexemeTagBase) raw.utf8ByteSize
        let fmt := applyLayoutMarks source lengths (lines.contains i) fmt
        let doc := document (restoreLexemes (Std.HashMap.ofList entry.lexemes.toList) fmt)
        memo.modify fun m => { m with documents := m.documents.insert documentKey doc }
        docs := docs.push doc
      | .error message =>
        docs := docs.push (document (.text original.pretty))
        kept := kept.push (keepIt message)
  return (docs, kept)

def appendJson (a b : Json) : Json := Json.arr #[toJson "append", a, b]

def joinDocuments (source : String) (commands : Array Syntax) (docs : Array Json) : Json := Id.run do
  let mut out := Json.arr #[toJson "nil"]
  for doc in docs, i in [0:docs.size] do
    if i > 0 then
      let gap := String.Pos.Raw.extract source (commands[i - 1]!.getTailPos?.getD 0)
        (commands[i]!.getPos?.getD 0)
      out := appendJson out (Json.arr #[toJson "line"])
      if blankLineBetween gap then out := appendJson out (Json.arr #[toJson "line"])
    out := appendJson out (appendJson (Json.arr #[toJson "command", toJson i]) doc)
  return out

def commandRange (cmd : Syntax) : Json :=
  Json.arr #[toJson (cmd.getPos?.getD 0).byteIdx, toJson (cmd.getTailPos?.getD 0).byteIdx]

structure Request where
  source : String
  verbatim : Array Nat := #[]
  lines : Array Nat := #[]
  document : Bool := true
  session : Nat := 0
  deriving FromJson

structure Run where
  source : ByteArray
  entries : Array Entry

def commonPrefix (a b : ByteArray) : Nat := Id.run do
  let n := min a.size b.size
  let mut i := 0
  while i < n && a.get! i == b.get! i do i := i + 1
  return i

abbrev Parts := Array (ModuleData × CompactedRegion)

structure LoadedModule extends EffectiveImport where
  parts : Parts
  irParts : Parts
  needsIRTrans : Bool

structure LoadedState where
  moduleNameMap : Std.HashMap Name LoadedModule := {}
  moduleNames : Array Name := #[]

structure Loaded where
  data : Std.HashMap Name Parts := {}
  ir : Std.HashMap Name Parts := {}

def LoadedModule.mainModule? (m : LoadedModule) : Option ModuleData :=
  if m.hasData then
    let isModule := m.parts[0]?.any (·.1.isModule)
    let level : OLeanLevel := if isModule && m.importAll then .private else .exported
    m.parts[level.ctorIdx]?.map (·.1)
  else
    m.irParts.back?.map (·.1)

unsafe def readData (loaded : IO.Ref Loaded) (mod : Name) : IO Parts := do
  if let some parts := (← loaded.get).data[mod]? then return parts
  let mFile ← findOLean mod
  unless (← mFile.pathExists) do
    throw <| IO.userError s!"object file '{mFile}' of module {mod} does not exist"
  let main ← CompactedRegion.read (α := ModuleData) mFile #[]
  let parts ← if !main.1.isModule then pure #[main] else do
    let server ← CompactedRegion.read (α := ModuleData)
      (OLeanLevel.server.adjustFileName mFile) #[main.2]
    let priv ← CompactedRegion.read (α := ModuleData)
      (OLeanLevel.private.adjustFileName mFile) #[main.2, server.2]
    pure #[main, server, priv]
  loaded.modify fun l => { l with data := l.data.insert mod parts }
  return parts

unsafe def readIR (loaded : IO.Ref Loaded) (mod : Name) : IO Parts := do
  if let some parts := (← loaded.get).ir[mod]? then return parts
  let mFile ← findOLean mod
  let irSigFile := mFile.withExtension "ir.sig"
  let parts ← if !(← irSigFile.pathExists) then pure #[] else do
    let irSig ← CompactedRegion.read (α := ModuleData) irSigFile #[]
    let ir ← CompactedRegion.read (α := ModuleData) (mFile.withExtension "ir") #[irSig.2]
    pure #[irSig, ir]
  loaded.modify fun l => { l with ir := l.ir.insert mod parts }
  return parts

unsafe def loadImports (loaded : IO.Ref Loaded) (globalLevel : OLeanLevel)
    (imports : Array Import) : StateRefT LoadedState IO Unit :=
  go imports true (globalLevel < .private) true false
where
  go (imports : Array Import) (importAll isExported needsData needsIRTrans : Bool) :
      StateRefT LoadedState IO Unit := do
    for i in imports do
      let needsData := needsData && (i.isExported || importAll)
      let importAll := globalLevel == .private || importAll && i.importAll
      let isExported := isExported && i.isExported
      let needsIRTrans := needsIRTrans || (needsData && i.isMeta)
      let needsIR := needsIRTrans || importAll || globalLevel > .exported
      if !needsData && !needsIR then
        continue
      let irPhases : IRPhases :=
        if importAll then .all else if needsIRTrans then .comptime else .runtime
      let goRec (mod : LoadedModule) := do
        if let some data := mod.mainModule? then
          go data.imports importAll isExported needsData needsIRTrans
      if let some mod := (← get).moduleNameMap[i.module]? then
        let importAll := importAll || mod.importAll
        let isExported := isExported || mod.isExported
        let needsData := needsData || mod.hasData
        let needsIRTrans := needsIRTrans || mod.needsIRTrans
        let needsIR := needsIRTrans || importAll
        let irPhases := if irPhases == mod.irPhases then irPhases else .all
        let parts ← if needsData && mod.parts.isEmpty then readData loaded i.module
          else pure mod.parts
        let irParts ← if needsIR && mod.irParts.isEmpty then readIR loaded i.module
          else pure mod.irParts
        if importAll != mod.importAll || isExported != mod.isExported ||
            needsIRTrans != mod.needsIRTrans || needsData != mod.hasData ||
            irPhases != mod.irPhases then
          let mod := { mod with
            importAll, isExported, irPhases, parts, irParts, hasData := needsData, needsIRTrans }
          modify fun s => { s with moduleNameMap := s.moduleNameMap.insert i.module mod }
          goRec mod
        continue
      let parts ← if needsData then readData loaded i.module else pure #[]
      let irParts ← if needsIR then readIR loaded i.module else pure #[]
      let mod : LoadedModule :=
        { i with importAll, isExported, irPhases, parts, irParts, needsIRTrans, hasData := needsData }
      goRec mod
      modify fun s => { s with
        moduleNameMap := s.moduleNameMap.insert i.module mod
        moduleNames := s.moduleNames.push i.module }

unsafe def importShared (loaded : IO.Ref Loaded) (imports : Array Import) (level : OLeanLevel) :
    IO Environment := do
  let ((), s) ← (loadImports loaded level imports).run {}
  if level < .private then
    for i in imports do
      if let some data := s.moduleNameMap[i.module]?.bind (·.mainModule?) then
        if !data.isModule then
          throw <| IO.userError s!"cannot import non-`module` {i.module} from `module`"
  enableInitializersExecution
  withImporting <| finalizeImport (unsafeCast s) imports {} 0 (leakEnv := false) (loadExts := true)
    (level := level)

unsafe def formatRequest (loaded : IO.Ref Loaded) (cache : IO.Ref (Option (String × Environment))) (memo : IO.Ref Memo)
    (runs : IO.Ref (Array Run)) (session : IO.Ref Nat) (req : Request) : IO Json := do
  if req.session != (← session.get) then
    session.set req.session
    if (← memo.get).formats.size > 100000 then memo.set {}
  let source := req.source
  let bytes := source.toUTF8
  let ctx := Parser.mkInputContext source "input.lean"
  let (header, ps, messages) ← Parser.parseHeader ctx
  if messages.hasErrors then
    let texts ← messages.toList.mapM (·.toString)
    throw (IO.userError (String.intercalate "\n" texts ++ "\ninvalid module header"))
  -- Match the compiler: module imports do not require private dependency artifacts.
  let level := if Elab.HeaderSyntax.isModule header then .exported else .private
  let imports := Elab.headerToImports header
  let key := toString (repr imports) ++ toString (Elab.HeaderSyntax.isModule header)
  let cached := match ← cache.get with
    | some (cached, env) => if cached == key then some env else none
    | none => none
  let env ← match cached with
    | some env => pure env
    | none => do
      cache.set none
      runs.set #[]
      let env ← importShared loaded imports level
      cache.set (some (key, env))
      pure env
  let mut reused : Array Entry := #[]
  let mut resume : Option Elab.Frontend.State := none
  for run in ← runs.get do
    let same := commonPrefix run.source bytes
    if same < ps.pos.byteIdx then continue
    let mut k := 0
    while run.entries[k + 1]?.any (·.stop <= same) do k := k + 1
    if same == bytes.size && same == run.source.size then k := run.entries.size
    if k > reused.size then
      reused := run.entries.extract 0 k
      resume := run.entries[k]?.map (·.before)
  let initial : Elab.Frontend.State := resume.getD
    { commandState := Elab.Command.mkState env {} (Options.setBool {} `quotPrecheck false),
      parserState := ps, cmdPos := ps.pos }
  let entries ← if reused.size > 0 && resume.isNone then pure reused else do
    let seed := hash (String.Pos.Raw.extract source 0 ps.pos)
    let (entries, _) ← (collect seed reused).run { inputCtx := ctx } |>.run initial
    pure entries
  runs.modify fun rs => (rs.filter (·.source != bytes)).push { source := bytes, entries } |>.extract
    (rs.size - 3) (rs.size + 1)
  let commands := entries.map (·.command)
  let (headerFmt, _) ← (PrettyPrinter.format Parser.Module.header.formatter (withoutTrivia header)).toIO
    { fileName := "input.lean", fileMap := ctx.fileMap } { env }
  let headerFmt := match header.raw.getTailPos?, commands[0]? with
    | some tail, some first =>
      if blankLineBetween (String.Pos.Raw.extract source tail (first.getPos?.getD tail)) then headerFmt
      else dropLastBreak headerFmt
    | _, _ => headerFmt
  let headerLexemes := Std.HashMap.ofList (lexemes source header).toList
  let (documentJson, kept) ← if !req.document then pure (Json.arr #[toJson "nil"], #[]) else do
    let (docs, kept) ← commandDocuments memo ctx req.verbatim req.lines (offRegions source) entries
    pure (appendJson (document (restoreLexemes headerLexemes headerFmt))
      (joinDocuments source commands docs), kept)
  let flat (f : Entry → Array Json) := entries.foldl (init := #[]) fun acc e => acc ++ f e
  return Json.mkObj [
    ("Trees", toJson (#[hash (fingerprint header).compress] ++ entries.map (·.tree))),
    ("Document", documentJson),
    ("Trivia", Json.arr (commentTrivia header ++ flat (·.trivia))),
    ("Tokens", Json.arr (tokens header ++ flat (·.tokens))),
    ("Blocks", Json.arr (blocks header ++ flat (·.blocks))),
    ("Commands", Json.arr (commands.map commandRange)), ("Kept", Json.arr kept),
    ("Lines", toJson (req.lines.filter (· < commands.size)))]

unsafe def main : IO Unit := do
  initSearchPath (← findSysroot)
  enableInitializersExecution
  let loaded ← IO.mkRef {}
  let cache ← IO.mkRef none
  let memo ← IO.mkRef {}
  let runs ← IO.mkRef #[]
  let session ← IO.mkRef 0
  let stdin ← IO.getStdin
  let stdout ← IO.getStdout
  repeat
    let line ← stdin.getLine
    if line.isEmpty then break
    let response ← try
        let req ← IO.ofExcept (Json.parse line >>= fromJson?)
        formatRequest loaded cache memo runs session req
      catch ex => pure (Json.mkObj [("Error", toJson ex.toString)])
    stdout.putStrLn response.compress
    stdout.flush
