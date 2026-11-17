leanfran::lean! {
    pub mod model;
    r#"
    inductive Status where
      | error | formatted | changed | unchanged
      deriving DecidableEq, Repr

    structure Outcome where
      failed : Bool
      changed : Bool
      deriving DecidableEq, Repr

    def minWidth : Nat := 20

    def maxWidth : Nat := 300

    def defaultWidth : Nat := 100

    def validWidth (width : Nat) : Bool := minWidth ≤ width && width ≤ maxWidth

    def lineWidth (flag configured : Option Nat) : Nat := flag.getD (configured.getD defaultWidth)

    def jobs (requested : Option Nat) (available : Nat) : Nat := max 1 (min 64 (requested.getD available))

    def workers (jobs groups : Nat) : Nat := min jobs (max groups 1)

    def writable (write check literal : Bool) : Bool := !(write && (check || literal))

    def writes (write changed : Bool) : Bool := write && changed

    def status (outcome : Outcome) (write : Bool) : Status :=
      if outcome.failed then .error
      else if writes write outcome.changed then .formatted
      else if outcome.changed then .changed
      else .unchanged

    def pending (write : Bool) (outcome : Outcome) : Bool := outcome.changed && !write

    def exitCode (check write : Bool) (outcomes : List Outcome) : Nat :=
      if outcomes.any (·.failed) || (check && outcomes.any (pending write)) then 254 else 0

    theorem default_width_is_valid : validWidth (lineWidth none none) = true := by decide

    theorem flag_overrides_configuration (width : Nat) (configured : Option Nat) :
        lineWidth (some width) configured = width := rfl

    theorem jobs_bounds (requested : Option Nat) (available : Nat) :
        1 ≤ jobs requested available ∧ jobs requested available ≤ 64 := by
      unfold jobs
      omega

    theorem workers_bounds (jobs groups : Nat) (h : 1 ≤ jobs) :
        1 ≤ workers jobs groups ∧ workers jobs groups ≤ jobs ∧ workers jobs groups ≤ max groups 1 := by
      unfold workers
      omega

    theorem check_never_writes (write literal : Bool) (h : writable write true literal = true) :
        write = false := by
      cases write <;> simp_all [writable]

    theorem exitCode_cases (check write : Bool) (outcomes : List Outcome) :
        exitCode check write outcomes = 0 ∨ exitCode check write outcomes = 254 := by
      unfold exitCode
      split <;> simp

    theorem check_passes_iff (outcomes : List Outcome) :
        exitCode true false outcomes = 0 ↔
          ∀ outcome ∈ outcomes, outcome.failed = false ∧ outcome.changed = false := by
      unfold exitCode
      constructor
      · intro h outcome hmem
        by_cases hf : outcome.failed = true
        · have hany : outcomes.any (·.failed) = true := List.any_eq_true.mpr ⟨outcome, hmem, hf⟩
          simp [hany] at h
        · by_cases hc : outcome.changed = true
          · have hany : outcomes.any (pending false) = true :=
              List.any_eq_true.mpr ⟨outcome, hmem, by simp [pending, hc]⟩
            simp [hany] at h
          · exact ⟨by simpa using hf, by simpa using hc⟩
      · intro h
        have hf : outcomes.any (·.failed) = false := by
          simp only [List.any_eq_false]
          intro outcome hmem
          simp [(h outcome hmem).1]
        have hc : outcomes.any (pending false) = false := by
          simp only [List.any_eq_false]
          intro outcome hmem
          simp [pending, (h outcome hmem).2]
        simp [hf, hc]

    theorem format_passes_iff (write : Bool) (outcomes : List Outcome) :
        exitCode false write outcomes = 0 ↔ ∀ outcome ∈ outcomes, outcome.failed = false := by
      simp only [exitCode]
      constructor
      · intro h outcome hmem
        by_cases hf : outcome.failed = true
        · simp [List.any_eq_true.mpr ⟨outcome, hmem, hf⟩] at h
        · simpa using hf
      · intro h
        have hf : outcomes.any (·.failed) = false := by
          simp only [List.any_eq_false]
          intro outcome hmem
          simp [h outcome hmem]
        simp [hf]

    theorem status_error_iff (outcome : Outcome) (write : Bool) :
        status outcome write = .error ↔ outcome.failed = true := by
      unfold status
      split
      · simp_all
      · split
        · simp_all
        · split <;> simp_all

    theorem status_formatted_iff (outcome : Outcome) (write : Bool) :
        status outcome write = .formatted ↔
          outcome.failed = false ∧ write = true ∧ outcome.changed = true := by
      unfold status writes
      cases outcome.failed <;> cases write <;> cases outcome.changed <;> simp
    "#
}

pub use model::{Outcome, Status, status, writable, writes};

pub fn min_width() -> usize {
    crate::index(model::min_width())
}

pub fn max_width() -> usize {
    crate::index(model::max_width())
}

pub fn valid_width(width: usize) -> bool {
    model::valid_width(crate::nat(width))
}

pub fn line_width(flag: Option<usize>, configured: Option<usize>) -> usize {
    crate::index(model::line_width(
        flag.map(crate::nat),
        configured.map(crate::nat),
    ))
}

pub fn jobs(requested: Option<usize>, available: usize) -> usize {
    crate::index(model::jobs(
        requested.map(crate::nat),
        crate::nat(available),
    ))
}

pub fn workers(jobs: usize, groups: usize) -> usize {
    crate::index(model::workers(crate::nat(jobs), crate::nat(groups)))
}

pub fn exit_code(check: bool, write: bool, outcomes: Vec<Outcome>) -> i32 {
    crate::index(model::exit_code(check, write, outcomes)) as i32
}
