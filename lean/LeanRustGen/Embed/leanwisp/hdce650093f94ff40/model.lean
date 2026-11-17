namespace model

structure File where
  index : Nat
  size : Nat
  deriving DecidableEq, Repr

def importCost : Nat := 32000

def size (group : List File) : Nat := group.foldl (fun sum file => sum + file.size) 0

def weight (group : List File) : Nat := importCost + size group

def addToLeast (loads : List Nat) (weight : Nat) : List Nat :=
  match loads.min? with
  | some least => loads.replace least (least + weight)
  | none => loads

def makespan (jobs : Nat) (groups : List (List File)) : Nat :=
  let weights := (groups.map weight).mergeSort fun a b => b ≤ a
  (weights.foldl addToLeast (List.replicate jobs 0)).foldl max 0

structure Halves where
  left : List File
  leftSize : Nat
  right : List File
  rightSize : Nat

def place (halves : Halves) (file : File) : Halves :=
  if halves.leftSize ≤ halves.rightSize then
    { halves with left := file :: halves.left, leftSize := halves.leftSize + file.size }
  else { halves with right := file :: halves.right, rightSize := halves.rightSize + file.size }

def halve (group : List File) : List File × List File :=
  let halves := (group.mergeSort fun a b => b.size ≤ a.size).foldl place ⟨[], 0, [], 0⟩
  (halves.left.reverse, halves.right.reverse)

def heaviestFrom (index : Nat) (best : Option (Nat × Nat)) : List (List File) → Option Nat
  | [] => best.map (·.1)
  | group :: rest =>
    match group with
    | _ :: _ :: _ =>
      match best with
      | some (found, heaviest) =>
        if heaviest ≤ weight group then heaviestFrom (index + 1) (some (index, weight group)) rest
        else heaviestFrom (index + 1) (some (found, heaviest)) rest
      | none => heaviestFrom (index + 1) (some (index, weight group)) rest
    | _ => heaviestFrom (index + 1) best rest

def heaviest (groups : List (List File)) : Option Nat := heaviestFrom 0 none groups

def swapRemove (groups : List (List File)) (index : Nat) : List (List File) :=
  match groups.getLast? with
  | some last =>
    if index + 1 = groups.length then groups.dropLast else groups.dropLast.set index last
  | none => groups

def split (groups : List (List File)) (index : Nat) : List (List File) :=
  match groups[index]? with
  | some group => swapRemove groups index ++ [(halve group).1, (halve group).2]
  | none => groups

def refine (jobs : Nat) : Nat → List (List File) → List (List File)
  | 0, groups => groups
  | fuel + 1, groups =>
    match heaviest groups with
    | some index =>
      if makespan jobs (split groups index) > makespan jobs groups then groups
      else refine jobs fuel (split groups index)
    | none => groups

def fileCount (groups : List (List File)) : Nat := groups.foldl (fun count group => count + group.length) 0

def order (groups : List (List File)) : List (List File) :=
  ((groups.map fun group => (weight group, group)).mergeSort fun a b => b.1 ≤ a.1).map (·.2)

def schedule (jobs : Nat) (groups : List (List File)) : List (List Nat) :=
  (order (refine jobs (fileCount groups) groups)).map fun group => group.map (·.index)

theorem refine_makespan (jobs : Nat) :
    ∀ fuel groups, makespan jobs (refine jobs fuel groups) ≤ makespan jobs groups := by
  intro fuel
  induction fuel with
  | zero => intro groups; exact Nat.le_refl _
  | succ fuel ih =>
    intro groups
    unfold refine
    split
    next index _ =>
      split
      · exact Nat.le_refl _
      · exact Nat.le_trans (ih (split groups index)) (by omega)
    next => exact Nat.le_refl _

theorem place_perm (halves : Halves) (file : File) :
    ((place halves file).left ++ (place halves file).right).Perm
      (file :: (halves.left ++ halves.right)) := by
  unfold place
  split
  · exact List.Perm.refl _
  · simp only
    exact List.perm_middle

theorem foldl_place_perm (files : List File) :
    ∀ halves, ((files.foldl place halves).left ++ (files.foldl place halves).right).Perm
      (files.reverse ++ (halves.left ++ halves.right)) := by
  induction files with
  | nil => intro halves; simp
  | cons file rest ih =>
    intro halves
    simp only [List.foldl_cons, List.reverse_cons, List.append_assoc, List.singleton_append]
    exact (ih (place halves file)).trans
      ((place_perm halves file).append_left rest.reverse)

theorem halve_perm (group : List File) : ((halve group).1 ++ (halve group).2).Perm group := by
  unfold halve
  simp only
  have h := foldl_place_perm (group.mergeSort fun a b => decide (b.size ≤ a.size)) ⟨[], 0, [], 0⟩
  simp only [List.append_nil] at h
  refine ((List.reverse_perm _).append (List.reverse_perm _)).trans ?_
  exact h.trans ((List.reverse_perm _).trans (List.mergeSort_perm _ _))

theorem set_flatten_perm (groups : List (List File)) (last : List File) :
    ∀ index, (h : index < groups.length) →
      ((groups.set index last).flatten ++ groups[index]).Perm (groups.flatten ++ last) := by
  induction groups with
  | nil => intro index h; simp at h
  | cons group rest ih =>
    intro index h
    cases index with
    | zero =>
      simp only [List.set_cons_zero, List.flatten_cons, List.getElem_cons_zero, List.append_assoc]
      have h1 : (last ++ (rest.flatten ++ group)).Perm ((rest.flatten ++ group) ++ last) :=
        List.perm_append_comm
      have h2 : ((rest.flatten ++ group) ++ last).Perm ((group ++ rest.flatten) ++ last) :=
        List.perm_append_comm.append_right last
      simpa [List.append_assoc] using h1.trans h2
    | succ index =>
      simp only [List.set_cons_succ, List.flatten_cons, List.getElem_cons_succ, List.append_assoc]
      exact (ih index (by simpa using h)).append_left group

theorem swapRemove_perm (groups : List (List File)) (index : Nat) (group : List File)
    (h : groups[index]? = some group) :
    ((swapRemove groups index).flatten ++ group).Perm groups.flatten := by
  unfold swapRemove
  split
  next last hlast =>
    have hne : groups ≠ [] := by
      intro hnil
      simp [hnil] at hlast
    have hsplit : groups.dropLast ++ [last] = groups := by
      have hget : groups.getLast hne = last := by
        rw [List.getLast?_eq_some_getLast hne] at hlast
        exact Option.some.inj hlast
      rw [← hget]
      exact List.dropLast_concat_getLast hne
    have hlen : groups.length = groups.dropLast.length + 1 := by
      conv => lhs; rw [← hsplit]
      simp
    have hindex : index < groups.length := (List.getElem?_eq_some_iff.mp h).1
    split
    next heq =>
      have hgroup : group = last := by
        have : (groups.dropLast ++ [last])[index]? = some group := by rw [hsplit]; exact h
        rw [List.getElem?_append_right (by omega)] at this
        have hz : index - groups.dropLast.length = 0 := by omega
        rw [hz] at this
        simpa using this.symm
      subst hgroup
      conv => rhs; rw [← hsplit]
      simp
    next hne =>
      have hlt : index < groups.dropLast.length := by omega
      have hgroup : groups.dropLast[index] = group := by
        have : (groups.dropLast ++ [last])[index]? = some group := by rw [hsplit]; exact h
        rw [List.getElem?_append_left hlt] at this
        exact (List.getElem?_eq_some_iff.mp this).2
      rw [← hgroup]
      conv => rhs; rw [← hsplit]
      simpa using set_flatten_perm groups.dropLast last index hlt
  next hnone =>
    have : groups = [] := by simpa using hnone
    subst this
    simp at h

theorem split_perm (groups : List (List File)) (index : Nat) :
    (split groups index).flatten.Perm groups.flatten := by
  unfold split
  split
  next group h =>
    simp only [List.flatten_append, List.flatten_cons, List.flatten_nil, List.append_nil]
    exact ((halve_perm group).append_left _).trans (swapRemove_perm groups index group h)
  next => exact List.Perm.refl _

theorem refine_perm (jobs : Nat) :
    ∀ fuel groups, (refine jobs fuel groups).flatten.Perm groups.flatten := by
  intro fuel
  induction fuel with
  | zero => intro groups; exact List.Perm.refl _
  | succ fuel ih =>
    intro groups
    unfold refine
    split
    next index _ =>
      split
      · exact List.Perm.refl _
      · exact (ih (split groups index)).trans (split_perm groups index)
    next => exact List.Perm.refl _

theorem order_perm (groups : List (List File)) : (order groups).Perm groups := by
  unfold order
  have h := (List.mergeSort_perm (groups.map fun group => (weight group, group))
    fun a b => decide (b.1 ≤ a.1)).map (·.2)
  simpa [List.map_map, Function.comp_def] using h

/-- Every file is assigned to exactly one group. -/
theorem schedule_perm (jobs : Nat) (groups : List (List File)) :
    (schedule jobs groups).flatten.Perm (groups.flatten.map (·.index)) := by
  unfold schedule
  rw [← List.map_flatten]
  exact (((order_perm _).flatten).trans (refine_perm jobs _ groups)).map _

theorem swapRemove_subset (groups : List (List File)) (index : Nat) :
    ∀ group ∈ swapRemove groups index, group ∈ groups := by
  intro group h
  unfold swapRemove at h
  split at h
  next last hlast =>
    have hlast : last ∈ groups := List.mem_of_getLast? hlast
    split at h
    · exact List.dropLast_subset groups h
    · rcases List.mem_or_eq_of_mem_set h with h | h
      · exact List.dropLast_subset groups h
      · exact h ▸ hlast
  next => exact h

theorem split_within (groups : List (List File)) (index : Nat) :
    ∀ group ∈ split groups index, ∃ origin ∈ groups, ∀ file ∈ group, file ∈ origin := by
  intro group h
  unfold split at h
  split at h
  next origin horigin =>
    have hmem : origin ∈ groups := List.mem_of_getElem? horigin
    have hsub : ∀ file, file ∈ (halve origin).1 ++ (halve origin).2 → file ∈ origin :=
      fun file hfile => (halve_perm origin).subset hfile
    simp only [List.mem_append, List.mem_cons, List.not_mem_nil, or_false] at h
    rcases h with h | h | h
    · exact ⟨group, swapRemove_subset groups index group h, fun _ hfile => hfile⟩
    · exact ⟨origin, hmem, fun file hfile => hsub file (by simp [← h, hfile])⟩
    · exact ⟨origin, hmem, fun file hfile => hsub file (by simp [← h, hfile])⟩
  next => exact ⟨group, h, fun _ hfile => hfile⟩

theorem refine_within (jobs : Nat) :
    ∀ fuel groups, ∀ group ∈ refine jobs fuel groups,
      ∃ origin ∈ groups, ∀ file ∈ group, file ∈ origin := by
  intro fuel
  induction fuel with
  | zero => intro groups group h; exact ⟨group, h, fun _ hfile => hfile⟩
  | succ fuel ih =>
    intro groups group h
    unfold refine at h
    split at h
    next index _ =>
      split at h
      · exact ⟨group, h, fun _ hfile => hfile⟩
      · have ⟨middle, hmiddle, hfiles⟩ := ih (split groups index) group h
        have ⟨origin, horigin, hsub⟩ := split_within groups index middle hmiddle
        exact ⟨origin, horigin, fun file hfile => hsub file (hfiles file hfile)⟩
    next => exact ⟨group, h, fun _ hfile => hfile⟩

/-- The files of a scheduled group come from one input group, so they share their imports. -/
theorem schedule_within (jobs : Nat) (groups : List (List File)) :
    ∀ indices ∈ schedule jobs groups,
      ∃ origin ∈ groups, ∀ index ∈ indices, ∃ file ∈ origin, file.index = index := by
  intro indices h
  unfold schedule at h
  simp only [List.mem_map] at h
  have ⟨group, hgroup, hindices⟩ := h
  have hgroup : group ∈ refine jobs (fileCount groups) groups := (order_perm _).subset hgroup
  have ⟨origin, horigin, hsub⟩ := refine_within jobs _ groups group hgroup
  refine ⟨origin, horigin, ?_⟩
  intro index hindex
  rw [← hindices] at hindex
  have ⟨file, hfile, heq⟩ := List.mem_map.mp hindex
  exact ⟨file, hsub file hfile, heq⟩

end model
