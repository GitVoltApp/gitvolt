//! The interactive rebase editor's state, pure (`ui/src/irebase/model.ts`, `rowDrag.ts`, `inspect.ts`,
//! `ChipColumn.tsx`'s sums): rows newest first, as the graph shows them, with their actions and edited messages;
//! the branch chips; the selection; and the opening plan, for Reset. Its rules are the core's `todo::build` rules, so
//! what the editor shows is what the rebase does.

use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowAction {
    Pick,
    Reword,
    Squash,
    Fixup,
    Drop,
    Edit,
}

pub const ACTIONS: [RowAction; 6] = [RowAction::Pick, RowAction::Reword, RowAction::Squash, RowAction::Fixup, RowAction::Drop, RowAction::Edit];

impl RowAction {
    pub fn of_key(key: &str) -> Option<Self> {
        Some(match key {
            "p" => RowAction::Pick,
            "r" => RowAction::Reword,
            "s" => RowAction::Squash,
            "f" => RowAction::Fixup,
            "d" => RowAction::Drop,
            "e" => RowAction::Edit,
            _ => return None,
        })
    }

    pub fn wire(self) -> &'static str {
        match self {
            RowAction::Pick => "pick",
            RowAction::Reword => "reword",
            RowAction::Squash => "squash",
            RowAction::Fixup => "fixup",
            RowAction::Drop => "drop",
            RowAction::Edit => "edit",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RowAction::Pick => "Pick",
            RowAction::Reword => "Reword",
            RowAction::Squash => "Squash",
            RowAction::Fixup => "Fixup",
            RowAction::Drop => "Drop",
            RowAction::Edit => "Edit",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            RowAction::Pick => "Keep this commit as is.",
            RowAction::Reword => "Keep the changes, edit the message.",
            RowAction::Squash => "Fold into the commit below, combining both messages.",
            RowAction::Fixup => "Fold into the commit below, keeping only its message.",
            RowAction::Drop => "Remove this commit.",
            RowAction::Edit => "Stop before this commit is made: its changes staged, its message in the box, to change, split or reword it.",
        }
    }

    fn folds(self) -> bool {
        matches!(self, RowAction::Squash | RowAction::Fixup)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorRow {
    pub oid: String,
    pub summary: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub action: RowAction,
    /// The new message; `None`: unchanged. On a fold target, the merged message.
    pub edited: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorChip {
    pub branch: String,
    /// The row it was put on. Where git leaves it is `chip_row`.
    pub at: String,
    /// Its row when the editor opened; `None`: added here.
    pub origin: Option<String>,
    /// Struck out: deleted once the rebase completes.
    pub deleted: bool,
    /// Why it can't move; `None`: free.
    pub locked: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Base {
    pub name: String,
    pub oid: String,
    pub summary: String,
    pub chips: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub rows: Vec<EditorRow>,
    pub chips: Vec<EditorChip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorState {
    pub branch: String,
    pub base: Base,
    pub merges: u32,
    pub expect: BTreeMap<String, String>,
    /// Every local branch: a new chip's name must be new.
    pub branches: Vec<String>,
    pub rows: Vec<EditorRow>,
    pub chips: Vec<EditorChip>,
    pub selected: Vec<String>,
    pub anchor: Option<String>,
    /// The opening plan (with its preset): Reset and `dirty`.
    pub initial: Plan,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preset {
    pub rows: HashMap<String, RowAction>,
    /// Moves the preset's Squash and Fixup rows right above this row, so they fold into it.
    pub gather: Option<String>,
}

impl Preset {
    fn key(&self) -> String {
        let mut rows: Vec<(&String, &str)> = self.rows.iter().map(|(k, v)| (k, v.wire())).collect();
        rows.sort();
        format!("{:?}{rows:?}", self.gather)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenRebase {
    pub branch: String,
    pub base: String,
    pub preset: Option<Preset>,
}

impl OpenRebase {
    /// The same branch, base and preset.
    pub fn same_target(&self, other: &OpenRebase) -> bool {
        self.branch == other.branch && self.base == other.base && self.preset.as_ref().map(Preset::key) == other.preset.as_ref().map(Preset::key)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grouping {
    /// A Squash or Fixup row's fold target; absent: nothing below to fold into.
    pub into: HashMap<String, String>,
    /// A target's folded rows, oldest first.
    pub folded: HashMap<String, Vec<String>>,
}

/// The core's `norm`: CRLF to LF, trailing whitespace off.
fn norm(m: &str) -> String {
    m.replace("\r\n", "\n").trim_end().to_string()
}

fn short(oid: &str) -> String {
    oid.chars().take(7).collect()
}

fn plan_rows(plan: &Value) -> Vec<EditorRow> {
    plan["rows"].as_array().map_or(Vec::new(), |rows| {
        rows.iter()
            .map(|r| {
                let text = |k: &str| r[k].as_str().unwrap_or_default().to_string();
                EditorRow { oid: text("oid"), summary: text("summary"), message: text("message"), author_name: text("authorName"), author_email: text("authorEmail"), action: RowAction::Pick, edited: None }
            })
            .collect()
    })
}

pub fn from_plan(plan: &Value, preset: Option<&Preset>) -> EditorState {
    let rows = plan_rows(plan);
    let chips: Vec<EditorChip> = plan["chips"].as_array().map_or(Vec::new(), |list| {
        list.iter()
            .map(|c| {
                let at = c["at"].as_str().unwrap_or_default().to_string();
                EditorChip { branch: c["branch"].as_str().unwrap_or_default().to_string(), origin: Some(at.clone()), at, deleted: false, locked: c["locked"].as_str().map(str::to_string) }
            })
            .collect()
    });
    let strings = |v: &Value| v.as_array().map_or(Vec::new(), |a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect::<Vec<_>>());
    let expect = plan["expect"].as_object().map_or(BTreeMap::new(), |m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect());
    let base = Base { name: plan["base"].as_str().unwrap_or_default().to_string(), oid: plan["baseOid"].as_str().unwrap_or_default().to_string(), summary: plan["baseSummary"].as_str().unwrap_or_default().to_string(), chips: strings(&plan["baseChips"]) };
    let mut s = EditorState { branch: plan["branch"].as_str().unwrap_or_default().to_string(), base, merges: plan["merges"].as_u64().unwrap_or(0) as u32, expect, branches: strings(&plan["branches"]), initial: Plan { rows: rows.clone(), chips: chips.clone() }, rows, chips, selected: Vec::new(), anchor: None };
    if let Some(p) = preset {
        apply_preset(&mut s, p);
    }
    s.initial = Plan { rows: s.rows.clone(), chips: s.chips.clone() };
    s
}

pub fn apply_preset(s: &mut EditorState, p: &Preset) {
    for r in &mut s.rows {
        if let Some(a) = p.rows.get(&r.oid) {
            r.action = *a;
        }
    }
    if let Some(g) = p.gather.as_ref().filter(|g| s.rows.iter().any(|r| r.oid == **g)) {
        let gathered = |r: &EditorRow| r.oid != *g && p.rows.contains_key(&r.oid) && r.action.folds();
        let moved: Vec<EditorRow> = s.rows.iter().filter(|r| gathered(r)).cloned().collect();
        let rest: Vec<EditorRow> = s.rows.iter().filter(|r| !gathered(r)).cloned().collect();
        let at = rest.iter().position(|r| r.oid == *g).unwrap_or(0);
        s.rows = rest[..at].iter().chain(moved.iter()).chain(rest[at..].iter()).cloned().collect();
    }
}

/// Squash and Fixup fold into the nearest older row that isn't dropped.
pub fn grouping(rows: &[EditorRow]) -> Grouping {
    let mut g = Grouping::default();
    let mut target: Option<String> = None;
    for r in rows.iter().rev() {
        if r.action == RowAction::Drop {
            continue;
        }
        if r.action.folds() {
            if let Some(t) = &target {
                g.into.insert(r.oid.clone(), t.clone());
                g.folded.entry(t.clone()).or_default().push(r.oid.clone());
            }
            continue;
        }
        target = Some(r.oid.clone());
        g.folded.entry(r.oid.clone()).or_default();
    }
    g
}

/// The core's `merged`: non-empty messages, trimmed, blank-line separated, `\n` at the end.
pub fn merged_message(messages: &[String]) -> String {
    format!("{}\n", messages.iter().map(|m| norm(m)).filter(|m| !m.is_empty()).collect::<Vec<_>>().join("\n\n"))
}

fn row<'a>(s: &'a EditorState, oid: &str) -> Option<&'a EditorRow> {
    s.rows.iter().find(|r| r.oid == oid)
}

/// A row's message as the rebase writes it: its edit; else, on a target with Squash rows folding in, the merged
/// default; else its own.
pub fn target_message(s: &EditorState, oid: &str, g: &Grouping) -> String {
    let Some(t) = row(s, oid) else { return String::new() };
    if let Some(e) = &t.edited {
        return e.clone();
    }
    let squashed: Vec<&EditorRow> = g.folded.get(oid).map_or(Vec::new(), |f| f.iter().filter_map(|o| row(s, o)).filter(|r| r.action == RowAction::Squash).collect());
    if squashed.is_empty() {
        return t.message.clone();
    }
    let mut all = vec![t.message.clone()];
    all.extend(squashed.iter().map(|r| r.edited.clone().unwrap_or_else(|| r.message.clone())));
    merged_message(&all)
}

/// Where git leaves a chip put on `at`: the first row from there down that isn't dropped (a folded row: its target);
/// the base when nothing below survives.
pub fn chip_row(s: &EditorState, at: &str, g: &Grouping) -> String {
    let Some(start) = s.rows.iter().position(|r| r.oid == at) else { return s.base.oid.clone() };
    for r in &s.rows[start..] {
        if r.action != RowAction::Drop {
            return g.into.get(&r.oid).cloned().unwrap_or_else(|| r.oid.clone());
        }
    }
    s.base.oid.clone()
}

/// The rebased branch's row: the effective top. `None`: every row dropped.
pub fn rebased_row(s: &EditorState, g: &Grouping) -> Option<String> {
    let top = s.rows.iter().find(|r| r.action != RowAction::Drop)?;
    Some(g.into.get(&top.oid).cloned().unwrap_or_else(|| top.oid.clone()))
}

pub fn set_actions(s: &mut EditorState, oids: &[String], action: RowAction) {
    for r in &mut s.rows {
        if oids.contains(&r.oid) {
            r.action = action;
        }
    }
}

pub const NOTHING_BELOW: &str = "Nothing below to fold into";

/// The row replayed first: the lowest one that isn't dropped.
pub fn bottom_row(s: &EditorState) -> Option<String> {
    s.rows.iter().rev().find(|r| r.action != RowAction::Drop).map(|r| r.oid.clone())
}

/// A user's pick of `action` for `oids`. Squash or Fixup with the bottom row among them folds the others into it:
/// with Squash it becomes Reword (`Ok(Some(oid))`: it takes the merged message); with Fixup it keeps its action and
/// its own message; on its own it is refused.
pub fn choose_action(s: &mut EditorState, oids: &[String], action: RowAction) -> Result<Option<String>, &'static str> {
    let bottom = bottom_row(s);
    let Some(bottom) = bottom.filter(|b| action.folds() && oids.contains(b)) else {
        set_actions(s, oids, action);
        return Ok(None);
    };
    let rest: Vec<String> = oids.iter().filter(|o| **o != bottom).cloned().collect();
    if rest.is_empty() {
        return Err(NOTHING_BELOW);
    }
    set_actions(s, &rest, action);
    if action == RowAction::Fixup {
        if row(s, &bottom).is_some_and(|r| r.action == RowAction::Fixup) {
            set_actions(s, std::slice::from_ref(&bottom), RowAction::Pick);
        }
        return Ok(None);
    }
    set_actions(s, std::slice::from_ref(&bottom), RowAction::Reword);
    Ok(Some(bottom))
}

/// Sets a row's message; one equal to what it would be anyway is no edit.
pub fn edit_message(s: &mut EditorState, oid: &str, text: &str) {
    for r in &mut s.rows {
        if r.oid == oid {
            r.edited = None;
        }
    }
    let g = grouping(&s.rows);
    if norm(text) == norm(&target_message(s, oid, &g)) {
        return;
    }
    for r in &mut s.rows {
        if r.oid == oid {
            r.edited = Some(format!("{}\n", norm(text)));
        }
    }
}

/// Ctrl+↑ (false) / Ctrl+↓ (true): each selected row moves a place, unless the place is the edge or another selected
/// row that couldn't move.
pub fn move_selected(s: &mut EditorState, down: bool) {
    let sel: HashSet<&String> = s.selected.iter().collect();
    let mut rows = s.rows.clone();
    if down {
        for i in (0..rows.len().saturating_sub(1)).rev() {
            if sel.contains(&rows[i].oid) && !sel.contains(&rows[i + 1].oid) {
                rows.swap(i, i + 1);
            }
        }
    } else {
        for i in 1..rows.len() {
            if sel.contains(&rows[i].oid) && !sel.contains(&rows[i - 1].oid) {
                rows.swap(i - 1, i);
            }
        }
    }
    s.rows = rows;
}

/// A group drag: the rows `oids`, in their order, land together at slot `to` of the rows left without them.
pub fn move_rows(s: &mut EditorState, oids: &[String], to: usize) {
    let group: Vec<EditorRow> = s.rows.iter().filter(|r| oids.contains(&r.oid)).cloned().collect();
    if group.is_empty() {
        return;
    }
    let rest: Vec<EditorRow> = s.rows.iter().filter(|r| !oids.contains(&r.oid)).cloned().collect();
    let at = to.min(rest.len());
    s.rows = rest[..at].iter().chain(group.iter()).chain(rest[at..].iter()).cloned().collect();
}

pub fn select(s: &mut EditorState, oid: &str, shift: bool, ctrl: bool) {
    let a = if shift { s.anchor.as_ref().and_then(|x| s.rows.iter().position(|r| r.oid == *x)) } else { None };
    let b = s.rows.iter().position(|r| r.oid == oid);
    if let (Some(a), Some(b)) = (a, b) {
        let range: Vec<String> = s.rows[a.min(b)..=a.max(b)].iter().map(|r| r.oid.clone()).collect();
        if ctrl {
            for o in range {
                if !s.selected.contains(&o) {
                    s.selected.push(o);
                }
            }
        } else {
            s.selected = range;
        }
        return;
    }
    if ctrl {
        match s.selected.iter().position(|o| o == oid) {
            Some(i) => {
                s.selected.remove(i);
            }
            None => s.selected.push(oid.to_string()),
        }
    } else {
        s.selected = vec![oid.to_string()];
    }
    s.anchor = Some(oid.to_string());
}

pub fn move_chip(s: &mut EditorState, branch: &str, oid: &str) {
    for c in &mut s.chips {
        if c.branch == branch && c.locked.is_none() {
            c.at = oid.to_string();
            c.deleted = false;
        }
    }
}

/// "+" on a row: a new branch there, or why it can't be added.
pub fn add_chip(s: &mut EditorState, branch: &str, oid: &str) -> Result<(), String> {
    let name = branch.trim();
    if let Some(bad) = crate::refs::branch_name_error(name) {
        return Err(bad);
    }
    if name == s.branch || s.branches.iter().any(|b| b == name) || s.chips.iter().any(|c| c.branch == name) {
        return Err(format!("{name} already exists"));
    }
    s.chips.push(EditorChip { branch: name.to_string(), at: oid.to_string(), origin: None, deleted: false, locked: None });
    Ok(())
}

/// ×: an existing chip is struck out, or brought back; an added one goes. A locked chip can't be removed.
pub fn remove_chip(s: &mut EditorState, branch: &str) {
    let Some(i) = s.chips.iter().position(|c| c.branch == branch && c.locked.is_none()) else { return };
    if s.chips[i].origin.is_none() {
        s.chips.remove(i);
    } else {
        s.chips[i].deleted = !s.chips[i].deleted;
    }
}

/// The chip menu's "Delete branch": an existing chip is struck out, an added one goes.
pub fn delete_chip(s: &mut EditorState, branch: &str) {
    if s.chips.iter().any(|c| c.branch == branch && c.locked.is_none() && !c.deleted) {
        remove_chip(s, branch);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipChange {
    None,
    Moved,
    Added,
    Deleted,
}

pub fn chip_change(c: &EditorChip) -> ChipChange {
    match &c.origin {
        None => ChipChange::Added,
        Some(_) if c.deleted => ChipChange::Deleted,
        Some(o) if *o == c.at => ChipChange::None,
        Some(_) => ChipChange::Moved,
    }
}

/// "Remove from this plan's changes" / "Restore": the chip as it opened; an added one goes.
pub fn revert_chip(s: &mut EditorState, branch: &str) {
    let Some(i) = s.chips.iter().position(|c| c.branch == branch) else { return };
    let c = &s.chips[i];
    if c.locked.is_some() || chip_change(c) == ChipChange::None {
        return;
    }
    match c.origin.clone() {
        None => {
            s.chips.remove(i);
        }
        Some(o) => {
            s.chips[i].at = o;
            s.chips[i].deleted = false;
        }
    }
}

pub fn reset(s: &mut EditorState) {
    s.rows = s.initial.rows.clone();
    s.chips = s.initial.chips.clone();
    s.selected.clear();
    s.anchor = None;
}

fn shape(rows: &[EditorRow], chips: &[EditorChip]) -> Vec<String> {
    let mut out: Vec<String> = rows.iter().map(|r| format!("{}|{}|{:?}", r.oid, r.action.wire(), r.edited)).collect();
    out.extend(chips.iter().map(|c| format!("{}|{}|{}", c.branch, c.at, c.deleted)));
    out
}

pub fn dirty(s: &EditorState) -> bool {
    shape(&s.rows, &s.chips) != shape(&s.initial.rows, &s.initial.chips)
}

/// What keeps Start disabled, first one shown; the core refuses the same.
pub fn problems(s: &EditorState, g: &Grouping) -> Vec<String> {
    let mut out = Vec::new();
    if s.rows.iter().all(|r| r.action == RowAction::Drop) {
        out.push("Every commit is dropped: keep at least one, or cancel the rebase".to_string());
    }
    for r in s.rows.iter().rev() {
        if r.action.folds() && !g.into.contains_key(&r.oid) {
            out.push(format!("Nothing below {} {} to squash it into", short(&r.oid), r.summary));
        }
        if let Some(t) = g.into.get(&r.oid)
            && row(s, t).is_some_and(|x| x.action == RowAction::Edit)
        {
            out.push(format!("{} can't fold into {}: an Edit row stops before the commits above it fold in", short(&r.oid), short(t)));
        }
        if r.edited.as_ref().is_some_and(|e| e.trim().is_empty()) {
            out.push(format!("Write a message for {}", short(&r.oid)));
        }
    }
    out
}

/// The InteractiveRebase request's rows and chips.
pub fn to_request(s: &EditorState) -> (Vec<Value>, Vec<Value>) {
    let g = grouping(&s.rows);
    let rows = s
        .rows
        .iter()
        .map(|r| {
            let squashed_in = g.folded.get(&r.oid).is_some_and(|f| f.iter().any(|o| row(s, o).is_some_and(|x| x.action == RowAction::Squash)));
            let message = if r.action == RowAction::Drop {
                None
            } else if squashed_in {
                Some(target_message(s, &r.oid, &g))
            } else {
                r.edited.clone()
            };
            match message {
                Some(m) => json!({ "oid": r.oid, "action": r.action.wire(), "message": m }),
                None => json!({ "oid": r.oid, "action": r.action.wire() }),
            }
        })
        .collect();
    let chips = s
        .chips
        .iter()
        .filter_map(|c| {
            if c.locked.is_some() {
                return c.origin.as_ref().map(|o| json!({ "branch": c.branch, "at": { "kind": "row", "oid": o } }));
            }
            if c.origin.is_none() {
                return (!c.deleted).then(|| json!({ "branch": c.branch, "at": { "kind": "new", "oid": chip_row(s, &c.at, &g) } }));
            }
            let at = if c.deleted { json!({ "kind": "delete" }) } else { json!({ "kind": "row", "oid": chip_row(s, &c.at, &g) }) };
            Some(json!({ "branch": c.branch, "at": at }))
        })
        .collect();
    (rows, chips)
}

/// "Reload" after RefMoved: the new plan, keeping what it can of `old`. Each commit still there keeps its action and
/// message, and the commits of both keep `old`'s order among themselves; new ones stay where the plan has them.
/// Chips keep their moves and deletes, and added ones stay, where their rows still exist.
pub fn reload(old: &EditorState, plan: &Value) -> EditorState {
    let mut fresh = from_plan(plan, None);
    let in_fresh: HashSet<String> = fresh.rows.iter().map(|r| r.oid.clone()).collect();
    let kept: Vec<&EditorRow> = old.rows.iter().filter(|r| in_fresh.contains(&r.oid)).collect();
    let mut k = 0;
    let by_oid: HashMap<String, EditorRow> = fresh.rows.iter().map(|r| (r.oid.clone(), r.clone())).collect();
    let was: HashSet<&String> = old.rows.iter().map(|r| &r.oid).collect();
    let rows: Vec<EditorRow> = fresh
        .rows
        .iter()
        .map(|f| {
            if !was.contains(&f.oid) {
                return f.clone();
            }
            let o = kept[k];
            k += 1;
            EditorRow { action: o.action, edited: o.edited.clone(), ..by_oid[&o.oid].clone() }
        })
        .collect();
    let base = fresh.base.oid.clone();
    let exists = |oid: &str| oid == base || rows.iter().any(|r| r.oid == oid);
    let mut chips: Vec<EditorChip> = fresh
        .chips
        .iter()
        .map(|c| {
            let o = old.chips.iter().find(|x| x.branch == c.branch);
            match o {
                Some(o) if c.locked.is_none() && exists(&o.at) => EditorChip { at: o.at.clone(), deleted: o.deleted, ..c.clone() },
                _ => c.clone(),
            }
        })
        .collect();
    chips.extend(old.chips.iter().filter(|c| c.origin.is_none() && exists(&c.at) && !fresh.branches.contains(&c.branch)).cloned());
    fresh.rows = rows;
    fresh.chips = chips;
    fresh
}

/// What conflict prediction depends on: the base, the order, and which rows apply (messages don't matter).
pub fn prediction_key(s: &EditorState) -> String {
    let rows: Vec<String> = s.rows.iter().map(|r| format!("{}:{}", r.oid, if r.action == RowAction::Drop { "d" } else { "p" })).collect();
    format!("{}|{}", s.base.oid, rows.join(","))
}

/// The row a selection change is about: the one added; of a Shift range, its end away from the anchor; after a
/// removal, the one shown if still selected.
pub fn focus_of(s: &EditorState, before: &[String], current: Option<&str>) -> Option<String> {
    let added: Vec<&String> = s.selected.iter().filter(|o| !before.contains(o)).collect();
    if added.len() == 1 {
        return Some(added[0].clone());
    }
    if added.len() > 1 {
        let at = |o: Option<&str>| s.rows.iter().position(|r| Some(r.oid.as_str()) == o).map_or(-1, |i| i as i64);
        let a = at(s.anchor.as_deref());
        return added.iter().max_by_key(|o| (at(Some(o.as_str())) - a).abs()).map(|o| (*o).clone());
    }
    if let Some(c) = current.filter(|c| s.selected.iter().any(|o| o == c)) {
        return Some(c.to_string());
    }
    s.selected.last().cloned()
}

// ---- A row drag's sums (`rowDrag.ts`) ----

fn group_above(heights: &[f32], group: &[usize], i: usize) -> f32 {
    group.iter().filter(|g| **g < i).map(|g| heights[*g]).sum()
}

/// The slot (among the other rows) the group lands in when the pressed row `from` is `dy` from where it was: the
/// block's top passes a row once it crosses that row's midpoint.
pub fn group_target(heights: &[f32], group: &[usize], from: usize, dy: f32) -> usize {
    let top = heights[..from].iter().sum::<f32>() + dy - group_above(heights, group, from);
    let mut y = 0.;
    let mut slot = 0;
    for (i, h) in heights.iter().enumerate() {
        if group.contains(&i) {
            continue;
        }
        if top <= y + h / 2. {
            break;
        }
        slot += 1;
        y += h;
    }
    slot
}

/// How far each row moves to show the order a release at slot `to` would make. A group row's entry is the offset of
/// its place in the landed block.
pub fn slot_shifts(heights: &[f32], group: &[usize], to: usize) -> Vec<f32> {
    let rest: Vec<usize> = (0..heights.len()).filter(|i| !group.contains(i)).collect();
    let mut out = vec![0.; heights.len()];
    let mut y = 0.;
    let at = to.min(rest.len());
    for i in rest[..at].iter().chain(group.iter()).chain(rest[at..].iter()) {
        out[*i] = y - heights[..*i].iter().sum::<f32>();
        y += heights[*i];
    }
    out
}

/// A dragged group row's offset from its own place: following the pointer, closed up around the pressed row.
pub fn group_offset(heights: &[f32], group: &[usize], from: usize, i: usize, dy: f32) -> f32 {
    let (a, b) = (i.min(from), i.max(from));
    let between: f32 = heights[a..b].iter().sum();
    let toward = if i < from { between } else { -between };
    dy + toward - group_above(heights, group, from) + group_above(heights, group, i)
}

// ---- The chip column's sums (`ChipColumn.tsx`) ----

pub const READABLE: usize = 6;
const CHAR_W: f32 = 7.5;
const CHIP_PAD: f32 = 10.;
const X_W: f32 = 17.;
const GAP: f32 = 6.;
const PILL_W: f32 = 30.;
/// The column less its "+"; the base's row has no "+".
pub const LIST_W: f32 = 201.;
pub const BASE_LIST_W: f32 = 220.;

fn min_width(branch: &str, has_x: bool) -> f32 {
    CHIP_PAD + branch.chars().count().min(READABLE) as f32 * CHAR_W + if has_x { X_W } else { 0. }
}

/// How many of `items` (name, has an ×) the column shows: all when they fit at their readable minimum, else as many
/// as fit beside the "+N" pill.
pub fn chips_shown(items: &[(String, bool)], avail: f32) -> usize {
    let widths: Vec<f32> = items.iter().map(|(b, x)| min_width(b, *x)).collect();
    let total = |k: usize| widths[..k].iter().sum::<f32>() + GAP * k.saturating_sub(1) as f32;
    if total(items.len()) <= avail {
        return items.len();
    }
    let mut k = items.len().saturating_sub(1);
    while k > 0 && total(k) + GAP + PILL_W > avail {
        k -= 1;
    }
    k
}

/// `flex: 1 1 0` with `max-width: max-content`: chips sharing a row share its width, a short name keeping its own; none
/// goes below its minimum. `wants`: each chip's full width and minimum.
pub fn share_widths(wants: &[(f32, f32)], avail: f32) -> Vec<f32> {
    let gaps = GAP * wants.len().saturating_sub(1) as f32;
    let mut left = (avail - gaps).max(0.);
    let mut out = vec![0.; wants.len()];
    let mut open: Vec<usize> = (0..wants.len()).collect();
    // Those that want no more than an equal share take what they want; the rest split what's left.
    loop {
        if open.is_empty() {
            break;
        }
        let share = left / open.len() as f32;
        let (small, rest): (Vec<usize>, Vec<usize>) = open.iter().partition(|i| wants[**i].0 <= share);
        if small.is_empty() {
            for i in &open {
                out[*i] = share.max(wants[*i].1).min(wants[*i].0);
            }
            break;
        }
        for i in small {
            out[i] = wants[i].0;
            left -= wants[i].0;
        }
        open = rest;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn plan() -> Value {
        let row = |o: &str, s: &str| json!({ "oid": o, "summary": s, "message": format!("{s}\n"), "authorName": "Ada", "authorEmail": "ada@example.com", "authorTime": 0, "upstream": false });
        json!({ "branch": "main", "base": "trunk", "baseOid": "base", "baseSummary": "Base", "baseChips": ["trunk"], "merges": 0, "behind": 0, "rows": [row("c", "Third"), row("b", "Second"), row("a", "First")], "chips": [{ "branch": "feature/x", "at": "b", "locked": null }, { "branch": "locked", "at": "a", "locked": "checked out in /w" }], "branches": ["main", "trunk", "feature/x", "locked"], "expect": { "refs/heads/main": "c" } })
    }

    fn st() -> EditorState {
        from_plan(&plan(), None)
    }

    fn oids(s: &EditorState) -> Vec<&str> {
        s.rows.iter().map(|r| r.oid.as_str()).collect()
    }

    fn act(s: &mut EditorState, oid: &str, a: RowAction) {
        set_actions(s, &[oid.to_string()], a);
    }

    #[test]
    fn a_plan_reads_newest_first_all_pick() {
        let s = st();
        assert_eq!(oids(&s), ["c", "b", "a"]);
        assert!(s.rows.iter().all(|r| r.action == RowAction::Pick));
        assert_eq!(s.chips[1].locked.as_deref(), Some("checked out in /w"));
        assert!(!dirty(&s));
    }

    #[test]
    fn folds_go_into_the_nearest_older_row_that_isnt_dropped() {
        let mut s = st();
        act(&mut s, "c", RowAction::Squash);
        act(&mut s, "b", RowAction::Drop);
        let g = grouping(&s.rows);
        assert_eq!(g.into.get("c").map(String::as_str), Some("a"));
        assert_eq!(rebased_row(&s, &g).as_deref(), Some("a"));
        assert_eq!(target_message(&s, "a", &g), "First\n\nThird\n");
    }

    #[test]
    fn a_fixup_keeps_only_the_targets_message() {
        let mut s = st();
        act(&mut s, "c", RowAction::Fixup);
        let g = grouping(&s.rows);
        assert_eq!(target_message(&s, "b", &g), "Second\n");
        let (rows, _) = to_request(&s);
        assert_eq!(rows[1], json!({ "oid": "b", "action": "pick" }));
    }

    #[test]
    fn a_chip_lands_on_the_first_row_that_survives_or_its_target() {
        let mut s = st();
        let g = grouping(&s.rows);
        assert_eq!(chip_row(&s, "b", &g), "b");
        act(&mut s, "b", RowAction::Drop);
        let g = grouping(&s.rows);
        assert_eq!(chip_row(&s, "b", &g), "a");
        act(&mut s, "a", RowAction::Drop);
        let g = grouping(&s.rows);
        assert_eq!(chip_row(&s, "b", &g), "base");
        let mut s = st();
        act(&mut s, "b", RowAction::Squash);
        let g = grouping(&s.rows);
        assert_eq!(chip_row(&s, "b", &g), "a");
    }

    #[test]
    fn squashing_the_bottom_row_rewords_it_and_alone_is_refused() {
        let mut s = st();
        assert_eq!(choose_action(&mut s, &["a".to_string()], RowAction::Squash), Err(NOTHING_BELOW));
        assert_eq!(choose_action(&mut s, &["b".to_string(), "a".to_string()], RowAction::Squash), Ok(Some("a".to_string())));
        assert_eq!(s.rows[2].action, RowAction::Reword);
        assert_eq!(s.rows[1].action, RowAction::Squash);
        let mut s = st();
        assert_eq!(choose_action(&mut s, &["b".to_string(), "a".to_string()], RowAction::Fixup), Ok(None));
        assert_eq!(s.rows[2].action, RowAction::Pick);
    }

    #[test]
    fn a_message_equal_to_what_it_would_be_is_no_edit() {
        let mut s = st();
        edit_message(&mut s, "a", "First\n\n");
        assert_eq!(s.rows[2].edited, None);
        edit_message(&mut s, "a", "New\r\nbody  ");
        assert_eq!(s.rows[2].edited.as_deref(), Some("New\nbody\n"));
        assert!(dirty(&s));
    }

    #[test]
    fn moves_keep_the_selection_together() {
        let mut s = st();
        s.selected = vec!["b".into(), "a".into()];
        move_selected(&mut s, false);
        assert_eq!(oids(&s), ["b", "a", "c"]);
        move_selected(&mut s, false);
        assert_eq!(oids(&s), ["b", "a", "c"], "at the edge");
        move_selected(&mut s, true);
        assert_eq!(oids(&s), ["c", "b", "a"]);
        move_rows(&mut s, &["c".to_string(), "a".to_string()], 1);
        assert_eq!(oids(&s), ["b", "c", "a"], "a scattered group closes up");
    }

    #[test]
    fn select_takes_clicks_shift_and_ctrl() {
        let mut s = st();
        select(&mut s, "c", false, false);
        assert_eq!(s.selected, ["c"]);
        select(&mut s, "a", true, false);
        assert_eq!(s.selected, ["c", "b", "a"], "a range from the anchor");
        select(&mut s, "b", false, true);
        assert_eq!(s.selected, ["c", "a"], "off");
        select(&mut s, "b", false, true);
        assert_eq!(s.selected, ["c", "a", "b"], "on again");
    }

    #[test]
    fn chips_move_add_strike_out_and_revert() {
        let mut s = st();
        move_chip(&mut s, "feature/x", "c");
        assert_eq!(chip_change(&s.chips[0]), ChipChange::Moved);
        move_chip(&mut s, "locked", "c");
        assert_eq!(s.chips[1].at, "a", "a locked chip stays");
        revert_chip(&mut s, "feature/x");
        assert_eq!(chip_change(&s.chips[0]), ChipChange::None);
        remove_chip(&mut s, "feature/x");
        assert_eq!(chip_change(&s.chips[0]), ChipChange::Deleted);
        remove_chip(&mut s, "feature/x");
        assert_eq!(chip_change(&s.chips[0]), ChipChange::None, "the × brings it back");
        assert_eq!(add_chip(&mut s, "main", "c"), Err("main already exists".to_string()));
        add_chip(&mut s, " fresh ", "c").unwrap();
        assert_eq!(chip_change(&s.chips[2]), ChipChange::Added);
        remove_chip(&mut s, "fresh");
        assert_eq!(s.chips.len(), 2, "an added one goes");
    }

    #[test]
    fn the_problems_are_the_cores() {
        let mut s = st();
        let g = grouping(&s.rows);
        assert!(problems(&s, &g).is_empty());
        act(&mut s, "a", RowAction::Squash);
        let g = grouping(&s.rows);
        assert_eq!(problems(&s, &g)[0], "Nothing below a First to squash it into");
        let mut s = st();
        act(&mut s, "a", RowAction::Edit);
        act(&mut s, "b", RowAction::Fixup);
        let g = grouping(&s.rows);
        assert_eq!(problems(&s, &g)[0], "b can't fold into a: an Edit row stops before the commits above it fold in");
        let mut s = st();
        for o in ["a", "b", "c"] {
            act(&mut s, o, RowAction::Drop);
        }
        let g = grouping(&s.rows);
        assert_eq!(problems(&s, &g)[0], "Every commit is dropped: keep at least one, or cancel the rebase");
    }

    #[test]
    fn the_request_says_each_chips_landing() {
        let mut s = st();
        act(&mut s, "b", RowAction::Drop);
        add_chip(&mut s, "new-one", "b").unwrap();
        s.chips[1].deleted = false;
        let (rows, chips) = to_request(&s);
        assert_eq!(rows[1], json!({ "oid": "b", "action": "drop" }));
        assert_eq!(chips[0], json!({ "branch": "feature/x", "at": { "kind": "row", "oid": "a" } }));
        assert_eq!(chips[1], json!({ "branch": "locked", "at": { "kind": "row", "oid": "a" } }));
        assert_eq!(chips[2], json!({ "branch": "new-one", "at": { "kind": "new", "oid": "a" } }));
    }

    #[test]
    fn a_reload_keeps_what_it_can() {
        let mut old = st();
        act(&mut old, "c", RowAction::Reword);
        edit_message(&mut old, "c", "Edited");
        move_rows(&mut old, &["a".to_string()], 0);
        move_chip(&mut old, "feature/x", "c");
        add_chip(&mut old, "added", "gone").unwrap();
        add_chip(&mut old, "kept", "a").unwrap();
        let mut p = plan();
        p["rows"].as_array_mut().unwrap().insert(0, json!({ "oid": "n", "summary": "New", "message": "New\n", "authorName": "A", "authorEmail": "a@example.com", "authorTime": 0, "upstream": false }));
        let s = reload(&old, &p);
        assert_eq!(oids(&s), ["n", "a", "c", "b"], "old's order among themselves; the new one where the plan has it");
        assert_eq!(s.rows[2].action, RowAction::Reword);
        assert_eq!(s.rows[2].edited.as_deref(), Some("Edited\n"));
        assert_eq!(s.chips[0].at, "c");
        assert!(s.chips.iter().any(|c| c.branch == "kept"));
        assert!(!s.chips.iter().any(|c| c.branch == "added"), "its row's gone");
    }

    #[test]
    fn a_preset_gathers_its_folds_above_the_target() {
        let mut preset = Preset::default();
        preset.rows.insert("c".to_string(), RowAction::Squash);
        preset.gather = Some("a".to_string());
        let s = from_plan(&plan(), Some(&preset));
        assert_eq!(oids(&s), ["b", "c", "a"]);
        assert!(!dirty(&s), "the preset is the opening plan");
        let mut s2 = s.clone();
        act(&mut s2, "b", RowAction::Drop);
        assert!(dirty(&s2));
        reset(&mut s2);
        assert_eq!(s2.rows, s.rows);
    }

    #[test]
    fn the_inspected_row_follows_the_selection() {
        let mut s = st();
        s.anchor = Some("c".into());
        s.selected = vec!["c".into(), "b".into(), "a".into()];
        assert_eq!(focus_of(&s, &["c".to_string()], None).as_deref(), Some("a"), "a range's far end");
        s.selected = vec!["c".into()];
        assert_eq!(focus_of(&s, &["c".to_string(), "b".to_string()], Some("c")).as_deref(), Some("c"));
        assert_eq!(focus_of(&s, &["c".to_string(), "b".to_string()], Some("b")).as_deref(), Some("c"));
    }

    #[test]
    fn a_drags_slot_is_past_a_rows_midpoint() {
        let h = [30., 30., 30., 30.];
        assert_eq!(group_target(&h, &[0], 0, 0.), 0);
        assert_eq!(group_target(&h, &[0], 0, 14.), 0);
        assert_eq!(group_target(&h, &[0], 0, 16.), 1);
        assert_eq!(group_target(&h, &[0], 0, 80.), 3);
        assert_eq!(group_target(&h, &[1, 2], 1, 0.), 1, "a group stays home");
        assert_eq!(group_target(&h, &[1, 2], 1, 20.), 2);
        assert_eq!(slot_shifts(&h, &[0], 2), [60., -30., -30., 0.]);
        assert_eq!(group_offset(&h, &[0, 2], 0, 2, 10.), 10. - 60. + 30., "closed up under the pressed row");
    }

    #[test]
    fn what_fits_shows_and_the_rest_goes_behind_the_pill() {
        let item = |n: &str| (n.to_string(), true);
        assert_eq!(chips_shown(&[item("a"), item("b")], LIST_W), 2);
        let many: Vec<_> = (0..6).map(|i| item(&format!("feature/{i}"))).collect();
        assert_eq!(chips_shown(&many, LIST_W), 2);
        assert_eq!(share_widths(&[(40., 20.), (500., 62.)], 201.), [40., 155.], "a short name keeps its width");
        assert_eq!(share_widths(&[(79., 62.), (79., 62.)], 201.), [79., 79.]);
    }
}
