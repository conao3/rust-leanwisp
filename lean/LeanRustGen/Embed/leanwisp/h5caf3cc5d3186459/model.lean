namespace model

structure Stamp where
  path : String
  size : Nat
  seconds : Nat
  nanos : Nat
  deriving DecidableEq, Repr

structure Import where
  module : String
  stamp : Option Stamp
  deriving DecidableEq, Repr

inductive Field where
  | text (value : String)
  | number (value bytes : Nat)
  | module (name : String)
  | stamp (stamp : Stamp)
  deriving DecidableEq, Repr

def importFields (entry : Import) : List Field :=
  match entry.stamp with
  | some stamp => [.module entry.module, .stamp stamp]
  | none => [.module entry.module]

def importsFields : List Import → List Field
  | [] => []
  | entry :: rest => importFields entry ++ importsFields rest

def fields (version adapter : String) (width : Nat) (source : String) (imports : List Import) :
    List Field :=
  .text version :: .text adapter :: .number width 8 :: .text source :: importsFields imports

def offset : UInt64 := 0xcbf29ce484222325

def mix (hash : UInt64) (byte : UInt8) : UInt64 := (hash ^^^ byte.toUInt64) * 0x100000001b3

def absorbText (hash : UInt64) (value : String) : UInt64 := mix (value.toUTF8.foldl mix hash) 0xff

def absorbNumber (hash : UInt64) (value bytes : Nat) : UInt64 :=
  mix ((List.range bytes).foldl (fun hash index => mix hash (value >>> (8 * index)).toUInt8) hash) 0xff

def absorb (hash : UInt64) : Field → UInt64
  | .text value => absorbText hash value
  | .number value bytes => absorbNumber hash value bytes
  | .module name => absorbText hash name
  | .stamp stamp => absorbNumber (absorbNumber (absorbText hash stamp.path) stamp.size 8)
      (stamp.seconds * 1000000000 + stamp.nanos) 16

def key (version adapter : String) (width : Nat) (source : String) (imports : List Import) : UInt64 :=
  (fields version adapter width source imports).foldl absorb offset

theorem importsFields_head (imports : List Import) (stamp : Stamp) (rest : List Field) :
    importsFields imports ≠ .stamp stamp :: rest := by
  cases imports with
  | nil => simp [importsFields]
  | cons entry tail =>
    unfold importsFields importFields
    split <;> simp

theorem importsFields_injective :
    ∀ first second : List Import, importsFields first = importsFields second → first = second := by
  intro first
  induction first with
  | nil =>
    intro second h
    cases second with
    | nil => rfl
    | cons entry tail =>
      unfold importsFields importFields at h
      split at h <;> simp at h
  | cons entry tail ih =>
    intro second h
    cases second with
    | nil =>
      unfold importsFields importFields at h
      split at h <;> simp at h
    | cons other others =>
      obtain ⟨module, stamp⟩ := entry
      obtain ⟨otherModule, otherStamp⟩ := other
      cases stamp <;> cases otherStamp <;>
        simp only [importsFields, importFields, List.cons_append, List.nil_append,
          List.cons.injEq, Field.module.injEq, Field.stamp.injEq] at h
      · rw [h.1, ih others h.2]
      · exact absurd h.2 (importsFields_head tail _ _)
      · exact absurd h.2.symm (importsFields_head others _ _)
      · rw [h.1, h.2.1, ih others h.2.2]

/-- The hashed fields determine every input of the key. -/
theorem fields_injective (version adapter source version' adapter' source' : String)
    (width width' : Nat) (imports imports' : List Import)
    (h : fields version adapter width source imports =
      fields version' adapter' width' source' imports') :
    version = version' ∧ adapter = adapter' ∧ width = width' ∧ source = source' ∧
      imports = imports' := by
  simp only [fields, List.cons.injEq, Field.text.injEq, Field.number.injEq, and_true] at h
  exact ⟨h.1, h.2.1, h.2.2.1, h.2.2.2.1, importsFields_injective _ _ h.2.2.2.2⟩

end model
