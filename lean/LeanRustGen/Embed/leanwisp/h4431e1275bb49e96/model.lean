namespace model

structure Keep where
  lines : List Nat
  verbatim : List Nat
  deriving DecidableEq, Repr

inductive Mode where
  | canonical | lineBreaks | source
  deriving DecidableEq, Repr

inductive Blame where
  | commands (indices : List Nat)
  | file
  deriving DecidableEq, Repr

def mode (keep : Keep) (index : Nat) : Mode :=
  if keep.verbatim.contains index then .source
  else if keep.lines.contains index then .lineBreaks
  else .canonical

def escalateOne (keep : Keep) (index : Nat) : Option Keep :=
  match mode keep index with
  | .canonical => some { keep with lines := keep.lines ++ [index] }
  | .lineBreaks => some { keep with verbatim := keep.verbatim ++ [index] }
  | .source => none

def escalate (keep : Keep) : List Nat → Option Keep
  | [] => some keep
  | index :: rest =>
    match escalateOne keep index with
    | some next => escalate next rest
    | none => none

def lineKept (applied verbatim : List Nat) : List Nat :=
  applied.filter fun index => !verbatim.contains index

def level (keep : Keep) (index : Nat) : Nat :=
  match mode keep index with
  | .canonical => 0
  | .lineBreaks => 1
  | .source => 2

def total (keep : Keep) : Nat → Nat
  | 0 => 0
  | count + 1 => total keep count + level keep count

def differingFrom (index : Nat) (found : List Nat) : List String → List String → List Nat
  | a :: as, b :: bs => differingFrom (index + 1) (if a = b then found else index :: found) as bs
  | _, _ => found.reverse

def differing (a b : List String) : List Nat := differingFrom 0 [] a b

def commandTrees (trees : List String) : List String := trees.drop 1

def blame (original other : List String) : Blame :=
  let a := commandTrees original
  let b := commandTrees other
  match differing a b with
  | [] =>
    if a.length = b.length || a.isEmpty || b.isEmpty then .file
    else .commands [min a.length b.length - 1]
  | first :: rest => if a.length = b.length then .commands (first :: rest) else .commands [first]

def syntaxVerdict (original other : List String) (tokens otherTokens : Nat) : Option Blame :=
  if original = other ∧ tokens = otherTokens then none else some (blame original other)

def firstDifferenceFrom (index : Nat) : List (List String) → List (List String) → Option Nat
  | a :: as, b :: bs => if a = b then firstDifferenceFrom (index + 1) as bs else some index
  | _, _ => none

def commentVerdict (original formatted : List String)
    (byCommand formattedByCommand : List (List String)) : Option Blame :=
  if original = formatted then none
  else
    match firstDifferenceFrom 0 byCommand formattedByCommand with
    | some index => some (.commands [index])
    | none => some .file

def commandAtFrom (index best offset : Nat) : List Nat → Nat
  | [] => best
  | start :: rest => commandAtFrom (index + 1) (if start ≤ offset then index else best) offset rest

def commandAt (starts : List Nat) (offset : Nat) : Nat := commandAtFrom 0 0 offset starts

def stabilityVerdict (starts : List Nat) (difference : Option Nat) : Option Blame :=
  difference.map fun offset => .commands [commandAt starts offset]

def parseBlame (starts : Option (List Nat)) (offset : Option Nat) : Blame :=
  match starts, offset with
  | some starts, some offset => .commands [commandAt starts offset]
  | _, _ => .file

def rangesFrom (length previous : Nat) : List (Nat × Nat) → Bool
  | [] => true
  | range :: rest =>
    previous ≤ range.1 && range.1 ≤ range.2 && range.2 ≤ length && rangesFrom length range.2 rest

def validRanges (length : Nat) (ranges : List (Nat × Nat)) : Bool := rangesFrom length 0 ranges

theorem escalateOne_none_iff (keep : Keep) (index : Nat) :
    escalateOne keep index = none ↔ mode keep index = .source := by
  unfold escalateOne
  split <;> simp_all

theorem escalateOne_level (keep next : Keep) (index : Nat)
    (h : escalateOne keep index = some next) : level next index = level keep index + 1 := by
  unfold escalateOne at h
  unfold level
  split at h
  next hm =>
    cases h
    have hv : index ∉ keep.verbatim := by
      unfold mode at hm
      split at hm <;> simp_all
    simp [mode, hv]
  next hm =>
    cases h
    simp [mode]
  next => cases h

theorem escalateOne_other (keep next : Keep) (index other : Nat)
    (h : escalateOne keep index = some next) (hne : other ≠ index) :
    mode next other = mode keep other := by
  unfold escalateOne at h
  split at h
  · cases h; simp [mode, hne]
  · cases h; simp [mode, hne]
  · cases h

theorem total_le (keep : Keep) (count : Nat) : total keep count ≤ 2 * count := by
  induction count with
  | zero => simp [total]
  | succ count ih =>
    have : level keep count ≤ 2 := by
      unfold level
      split <;> omega
    simp only [total]
    omega

theorem escalateOne_total (keep next : Keep) (index : Nat)
    (h : escalateOne keep index = some next) :
    ∀ count, index < count → total next count = total keep count + 1 := by
  have hother : ∀ other, other ≠ index → level next other = level keep other := by
    intro other hne
    simp [level, escalateOne_other keep next index other h hne]
  have hbelow : ∀ count, count ≤ index → total next count = total keep count := by
    intro count
    induction count with
    | zero => intro _; rfl
    | succ count ih =>
      intro hle
      simp only [total]
      rw [ih (by omega), hother count (by omega)]
  intro count
  induction count with
  | zero => intro hlt; omega
  | succ count ih =>
    intro hlt
    simp only [total]
    by_cases heq : count = index
    · subst heq
      rw [hbelow count (Nat.le_refl count), escalateOne_level keep next count h]
      omega
    · rw [ih (by omega), hother count heq]
      omega

theorem escalate_total (count : Nat) (indices : List Nat) :
    ∀ keep next, (∀ index ∈ indices, index < count) → escalate keep indices = some next →
      total next count = total keep count + indices.length := by
  induction indices with
  | nil =>
    intro keep next _ h
    simp [escalate] at h
    simp [h]
  | cons index rest ih =>
    intro keep next hall h
    unfold escalate at h
    split at h
    next middle hstep =>
      have h1 := escalateOne_total keep middle index hstep count (hall index (by simp))
      have h2 := ih middle next (fun i hi => hall i (by simp [hi])) h
      simp only [List.length_cons]
      omega
    next => cases h

theorem lineKept_mode (applied verbatim : List Nat) (index : Nat)
    (h : index ∈ lineKept applied verbatim) : mode ⟨applied, verbatim⟩ index = .lineBreaks := by
  simp [lineKept] at h
  simp [mode, h]

theorem differingFrom_append (a b : List String) :
    ∀ index found, differingFrom index found a b = found.reverse ++ differingFrom index [] a b := by
  induction a generalizing b with
  | nil => intro index found; simp [differingFrom]
  | cons x xs ih =>
    intro index found
    cases b with
    | nil => simp [differingFrom]
    | cons y ys =>
      unfold differingFrom
      rw [ih ys (index + 1) (if x = y then found else index :: found)]
      rw [ih ys (index + 1) (if x = y then [] else [index])]
      split <;> simp

theorem differingFrom_bound (a b : List String) :
    ∀ index i, i ∈ differingFrom index [] a b → index ≤ i ∧ i < index + a.length ∧ i < index + b.length := by
  induction a generalizing b with
  | nil => intro index i h; simp [differingFrom] at h
  | cons x xs ih =>
    intro index i h
    cases b with
    | nil => simp [differingFrom] at h
    | cons y ys =>
      unfold differingFrom at h
      rw [differingFrom_append] at h
      simp only [List.mem_append, List.length_cons] at *
      rcases h with h | h
      · split at h
        · simp at h
        · simp at h
          omega
      · have := ih ys (index + 1) i h
        omega

theorem differingFrom_nil (a b : List String) :
    ∀ index, a.length = b.length → differingFrom index [] a b = [] → a = b := by
  induction a generalizing b with
  | nil =>
    intro index hlen _
    cases b with
    | nil => rfl
    | cons y ys => simp at hlen
  | cons x xs ih =>
    intro index hlen h
    cases b with
    | nil => simp at hlen
    | cons y ys =>
      unfold differingFrom at h
      rw [differingFrom_append] at h
      simp only [List.append_eq_nil_iff, List.reverse_eq_nil_iff] at h
      have hxy : x = y := by
        by_cases hxy : x = y
        · exact hxy
        · simp [hxy] at h
      have := ih ys (index + 1) (by simpa using hlen) h.2
      simp [hxy, this]

theorem differingFrom_self (a : List String) : ∀ index, differingFrom index [] a a = [] := by
  induction a with
  | nil => intro index; rfl
  | cons x xs ih => intro index; simp [differingFrom, ih]

theorem blame_bound (original other : List String) (indices : List Nat)
    (h : blame original other = .commands indices) :
    ∀ index ∈ indices, index < (commandTrees original).length := by
  intro index hmem
  unfold blame at h
  simp only at h
  split at h
  next hd =>
    split at h
    · cases h
    · rename_i hcond
      cases h
      simp at hcond
      simp at hmem
      have : (commandTrees original).length ≠ 0 := by
        intro hz
        exact hcond.1.2 (List.length_eq_zero_iff.mp hz)
      omega
  next first rest hd =>
    have hall : ∀ i ∈ first :: rest, i < (commandTrees original).length := by
      intro i hi
      have := differingFrom_bound (commandTrees original) (commandTrees other) 0 i
        (by unfold differing at hd; rw [hd]; exact hi)
      omega
    split at h
    · cases h; exact hall index hmem
    · cases h
      simp at hmem
      subst hmem
      exact hall index (by simp)

theorem blame_self (trees : List String) : blame trees trees = .file := by
  simp [blame, differing, differingFrom_self]

theorem blame_file (original other : List String) (h : blame original other = .file) :
    commandTrees original = commandTrees other ∨ commandTrees original = [] ∨
      commandTrees other = [] := by
  unfold blame at h
  simp only at h
  split at h
  next hd =>
    split at h
    next hcond =>
      simp at hcond
      rcases hcond with (hlen | hempty) | hempty
      · exact Or.inl (differingFrom_nil _ _ 0 hlen hd)
      · exact Or.inr (Or.inl hempty)
      · exact Or.inr (Or.inr hempty)
    next => cases h
  next => split at h <;> cases h

theorem syntaxVerdict_none_iff (original other : List String) (tokens otherTokens : Nat) :
    syntaxVerdict original other tokens otherTokens = none ↔
      original = other ∧ tokens = otherTokens := by
  unfold syntaxVerdict
  split <;> simp_all

theorem commentVerdict_none_iff (original formatted : List String)
    (byCommand formattedByCommand : List (List String)) :
    commentVerdict original formatted byCommand formattedByCommand = none ↔
      original = formatted := by
  unfold commentVerdict
  split
  · simp_all
  · split <;> simp_all

theorem commandAtFrom_bound (offset : Nat) (starts : List Nat) :
    ∀ index best, commandAtFrom index best offset starts = best ∨
      (index ≤ commandAtFrom index best offset starts ∧
        commandAtFrom index best offset starts < index + starts.length) := by
  induction starts with
  | nil => intro index best; exact Or.inl rfl
  | cons start rest ih =>
    intro index best
    unfold commandAtFrom
    rcases ih (index + 1) (if start ≤ offset then index else best) with h | h
    · rw [h]
      split
      · exact Or.inr ⟨Nat.le_refl index, by simp⟩
      · exact Or.inl rfl
    · exact Or.inr ⟨by omega, by simp only [List.length_cons]; omega⟩

theorem commandAt_lt (starts : List Nat) (offset : Nat) (h : starts ≠ []) :
    commandAt starts offset < starts.length := by
  have hpos : 0 < starts.length := List.length_pos_iff.mpr h
  rcases commandAtFrom_bound offset starts 0 0 with h | h
  · unfold commandAt
    rw [h]
    exact hpos
  · unfold commandAt
    omega

theorem commandAtFrom_last (offset : Nat) (starts : List Nat) :
    ∀ index best i, (h : i < starts.length) → starts[i] ≤ offset →
      index + i ≤ commandAtFrom index best offset starts := by
  induction starts with
  | nil => intro index best i h; simp at h
  | cons start rest ih =>
    intro index best i h hle
    unfold commandAtFrom
    cases i with
    | zero =>
      simp at hle
      simp only [hle, ite_true]
      rcases commandAtFrom_bound offset rest (index + 1) index with h | h <;> omega
    | succ i =>
      have := ih (index + 1) (if start ≤ offset then index else best) i
        (by simpa using h) (by simpa using hle)
      omega

theorem commandAt_last (starts : List Nat) (offset i : Nat) (h : i < starts.length)
    (hle : starts[i] ≤ offset) : i ≤ commandAt starts offset := by
  have := commandAtFrom_last offset starts 0 0 i h hle
  unfold commandAt
  omega

theorem rangesFrom_bounds (length : Nat) (ranges : List (Nat × Nat)) :
    ∀ previous, rangesFrom length previous ranges = true →
      ∀ range ∈ ranges, previous ≤ range.1 ∧ range.1 ≤ range.2 ∧ range.2 ≤ length := by
  induction ranges with
  | nil => intro previous _ range h; simp at h
  | cons first rest ih =>
    intro previous h range hmem
    simp only [rangesFrom, Bool.and_eq_true, decide_eq_true_eq] at h
    rcases List.mem_cons.mp hmem with hmem | hmem
    · subst hmem
      exact ⟨h.1.1.1, h.1.1.2, h.1.2⟩
    · have := ih first.2 h.2 range hmem
      omega

theorem rangesFrom_sorted (length : Nat) (ranges : List (Nat × Nat)) :
    ∀ previous, rangesFrom length previous ranges = true →
      ranges.Pairwise fun earlier later => earlier.2 ≤ later.1 := by
  induction ranges with
  | nil => intro previous _; exact List.Pairwise.nil
  | cons first rest ih =>
    intro previous h
    simp only [rangesFrom, Bool.and_eq_true, decide_eq_true_eq] at h
    exact List.Pairwise.cons
      (fun later hmem => (rangesFrom_bounds length rest first.2 h.2 later hmem).1)
      (ih first.2 h.2)

/-- Accepted token ranges lie inside the source, in order and without overlap. -/
theorem validRanges_sound (length : Nat) (ranges : List (Nat × Nat))
    (h : validRanges length ranges = true) :
    (∀ range ∈ ranges, range.1 ≤ range.2 ∧ range.2 ≤ length) ∧
      ranges.Pairwise fun earlier later => earlier.2 ≤ later.1 :=
  ⟨fun range hmem => (rangesFrom_bounds length ranges 0 h range hmem).2,
    rangesFrom_sorted length ranges 0 h⟩

end model
