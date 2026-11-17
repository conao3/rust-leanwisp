namespace model

inductive Doc where
  | nil
  | text (value : String)
  | line
  | softblock
  | attrline
  | hardBreak (delta : Int)
  | block (delta : Int)
  | align (force : Bool)
  | command
  | fieldcolumn
  | append (first second : Doc)
  | nest (indent : Int) (body : Doc)
  | group (body : Doc) (fill : Bool)
  deriving Repr

structure Space where
  line : Bool
  hard : Bool
  finish : Bool
  size : Nat
  deriving DecidableEq, Repr

def Space.empty : Space := ⟨false, false, false, 0⟩

def firstLine (value : String) : Nat := (value.toList.takeWhile (· != '\n')).length

def measure (flat : Bool) (margin : Int) : Doc → Nat → Space
  | .text value, _ =>
    let line := value.contains '\n'
    ⟨line, flat && line, false, firstLine value⟩
  | .command, _ => ⟨true, false, true, 0⟩
  | .line, _ | .softblock, _ | .attrline, _ => ⟨!flat, false, false, if flat then 1 else 0⟩
  | .hardBreak _, _ => ⟨true, flat, false, 0⟩
  | .block _, _ => ⟨true, false, false, 0⟩
  | .align force, width =>
    if !flat || force then
      if margin < width then ⟨false, false, false, (width - margin).toNat⟩
      else ⟨true, false, false, 0⟩
    else Space.empty
  | .append first second, width =>
    let head := measure flat margin first width
    if head.line || width < head.size then head
    else
      let tail := measure flat margin second (width - head.size)
      { tail with size := head.size + tail.size }
  | .nest indent body, width => measure flat (margin - indent) body width
  | .group body _, width => measure true margin body width
  | .nil, _ | .fieldcolumn, _ => Space.empty

def softblockIndent (indent : Int) (base : Nat) : Nat := min indent.toNat (base + 2)

def blockIndent (indent : Int) (base : Nat) (delta : Int) : Nat :=
  min (min (base + 2) indent.toNat) (base + delta).toNat

def breakIndent (base : Nat) (delta : Int) : Nat := (base + delta).toNat

def fits (hard : Bool) (total room : USize) : Bool := !hard && total ≤ room

def needsContext (line : Bool) (size room : USize) : Bool := !(line || room < size)

inductive Align where
  | indent
  | pad (spaces : Nat)
  | wrap
  deriving DecidableEq, Repr

def alignAction (column : Nat) (indent : Int) : Align :=
  if column = 0 then .indent
  else if column < indent.toNat then .pad (indent.toNat - column)
  else .wrap

theorem softblockIndent_le (indent : Int) (base : Nat) :
    softblockIndent indent base ≤ base + 2 ∧ softblockIndent indent base ≤ indent.toNat := by
  unfold softblockIndent
  omega

theorem blockIndent_le (indent : Int) (base : Nat) (delta : Int) :
    blockIndent indent base delta ≤ base + 2 ∧ blockIndent indent base delta ≤ indent.toNat ∧
      blockIndent indent base delta ≤ (base + delta).toNat := by
  unfold blockIndent
  omega

theorem breakIndent_zero (base : Nat) : breakIndent base 0 = base := by
  simp [breakIndent]

theorem fits_iff (hard : Bool) (total room : USize) :
    fits hard total room = true ↔ hard = false ∧ total ≤ room := by
  simp [fits]

theorem alignAction_pad (column : Nat) (indent : Int) (spaces : Nat)
    (h : alignAction column indent = .pad spaces) : 0 < spaces ∧ column + spaces = indent.toNat := by
  unfold alignAction at h
  split at h
  · cases h
  · split at h
    · cases h; omega
    · cases h

theorem measure_hard_line (doc : Doc) : ∀ flat margin width,
    (measure flat margin doc width).hard = true → (measure flat margin doc width).line = true := by
  induction doc with
  | append first second ihFirst ihSecond =>
    intro flat margin width
    simp only [measure]
    split
    · exact ihFirst flat margin width
    · exact ihSecond flat margin _
  | nest indent body ih => intro flat margin width; exact ih flat _ width
  | group body fill ih => intro flat margin width; exact ih true margin width
  | align force =>
    intro flat margin width
    simp only [measure]
    split
    · split <;> simp
    · simp [Space.empty]
  | _ => intro flat margin width; simp [measure, Space.empty]

theorem measure_finish_line (doc : Doc) : ∀ flat margin width,
    (measure flat margin doc width).finish = true → (measure flat margin doc width).line = true := by
  induction doc with
  | append first second ihFirst ihSecond =>
    intro flat margin width
    simp only [measure]
    split
    · exact ihFirst flat margin width
    · exact ihSecond flat margin _
  | nest indent body ih => intro flat margin width; exact ih flat _ width
  | group body fill ih => intro flat margin width; exact ih true margin width
  | align force =>
    intro flat margin width
    simp only [measure]
    split
    · split <;> simp
    · simp [Space.empty]
  | _ => intro flat margin width; simp [measure, Space.empty]

theorem measure_group (flat : Bool) (margin : Int) (body : Doc) (fill : Bool) (width : Nat) :
    measure flat margin (.group body fill) width = measure true margin body width := rfl

theorem measure_nil_append (flat : Bool) (margin : Int) (doc : Doc) (width : Nat) :
    measure flat margin (.append .nil doc) width = measure flat margin doc width := by
  simp [measure, Space.empty]

theorem measure_append_assoc (flat : Bool) (margin : Int) (a b c : Doc) (width : Nat) :
    measure flat margin (.append (.append a b) c) width =
      measure flat margin (.append a (.append b c)) width := by
  simp only [measure]
  generalize measure flat margin a width = A
  by_cases h1 : A.line = true ∨ width < A.size
  · simp [h1]
  · have hle : A.size ≤ width := by omega
    simp only [Bool.or_eq_true, decide_eq_true_eq, h1, ite_false]
    generalize measure flat margin b (width - A.size) = B
    by_cases h2 : B.line = true ∨ width - A.size < B.size
    · have h3 : B.line = true ∨ width < A.size + B.size := by
        rcases h2 with h | h
        · exact Or.inl h
        · exact Or.inr (by omega)
      simp [h2, h3]
    · have h3 : ¬ (B.line = true ∨ width < A.size + B.size) := by
        intro h
        rcases h with h | h
        · exact h2 (Or.inl h)
        · exact h2 (Or.inr (by omega))
      simp [h2, h3, Nat.sub_sub, Nat.add_assoc]

end model
