//! The sizes a drag sets, remembered: the sidebar's width and panels, the details panel's width and
//! the graph's columns. The web UI keeps them in the profile; until this build has profiles they're a
//! small file in its own cache folder.

use crate::app::{GitVolt, Tab};
use crate::model::{Col, ColumnPrefs, HIDEABLE};
use crate::sidebar::SectionKind;
use serde_json::{Value, json};
use std::path::PathBuf;

const KINDS: [SectionKind; 7] = [SectionKind::Gitflow, SectionKind::Local, SectionKind::Remote, SectionKind::Worktrees, SectionKind::Stashes, SectionKind::Tags, SectionKind::Submodules];

fn file() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(gitvolt_core::paths::cache_base()?.join("gitvolt-gpui").join("layout.json"))
}

fn col_key(c: Col) -> &'static str {
    match c {
        Col::Labels => "labels",
        Col::Graph => "graph",
        Col::Message => "message",
        Col::Author => "author",
        Col::Date => "date",
        Col::Sha => "sha",
    }
}

impl GitVolt {
    /// What the last run left, once at the start.
    pub fn load_layout(&mut self) {
        let Some(v) = file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return };
        self.apply_layout(&v);
    }

    pub fn apply_layout(&mut self, v: &Value) {
        if let Some(w) = v["sidebarWidth"].as_f64() {
            self.sidebar_w = (w as f32).clamp(crate::sidebar::MIN_W, crate::sidebar::MAX_W);
        }
        self.sidebar_narrow = v["sidebarNarrow"].as_bool().unwrap_or(false);
        if let Some(w) = v["detailsWidth"].as_f64() {
            self.details_w = (w as f32).clamp(crate::details::DETAILS_MIN, crate::details::DETAILS_MAX);
        }
        self.repo_order = if v["yourReposOrder"] == "alpha" { crate::open::RepoOrder::Alpha } else { crate::open::RepoOrder::Recent };
        self.repo_grouped = v["yourReposLayout"] == "grouped";
        self.branch_order_recent = v["branchPickerOrder"] == "recent";
        if let Some(r) = v["detailsSplit"].as_f64() {
            self.details_split = (r as f32).clamp(0.1, 0.75);
        }
        if let Some(r) = v["wipRatio"].as_f64() {
            self.wip_ratio = (r as f32).clamp(0.05, 0.95);
        }
        if let Some(c) = v["wipCollapsed"].as_array() {
            for (i, on) in c.iter().take(3).enumerate() {
                self.wip_closed[i] = on.as_bool() == Some(true);
            }
        }
        if let Some(m) = v["syncButton"].as_str() {
            self.sync_mode = crate::sync::SyncMode::of(m);
        }
        if let Some(m) = v["prefs"].as_object() {
            for (k, x) in m {
                self.prefs[k] = x.clone();
            }
        }
        crate::metrics::set_density(self.prefs["density"].as_str().unwrap_or("standard"));
        self.saved_columns = v["columns"].clone();
        for kind in KINDS {
            if let Some(h) = v["sidebarPanels"][kind.key()].as_f64() {
                self.side_weights.insert(kind, h as f32);
            }
        }
    }

    pub fn layout_json(&self) -> Value {
        let panels: serde_json::Map<String, Value> = self.side_weights.iter().map(|(k, h)| (k.key().to_string(), json!(h))).collect();
        let mut columns = self.saved_columns.as_object().cloned().unwrap_or_default();
        for tab in &self.tabs {
            let p = &tab.column_prefs;
            let hidden: Vec<&str> = HIDEABLE.iter().filter(|(c, _)| tab.hidden_columns.contains(c)).map(|(c, _)| col_key(*c)).collect();
            columns.insert(tab.worktree.clone(), json!({ "labels": p.labels, "graph": p.graph, "author": p.author, "date": p.date, "sha": p.sha, "message": p.message, "hidden": hidden }));
        }
        json!({ "prefs": self.prefs, "sidebarWidth": self.sidebar_w, "sidebarNarrow": self.sidebar_narrow, "sidebarPanels": panels, "branchPickerOrder": if self.branch_order_recent { "recent" } else { "alpha" }, "yourReposOrder": if self.repo_order == crate::open::RepoOrder::Alpha { "alpha" } else { "recent" }, "yourReposLayout": if self.repo_grouped { "grouped" } else { "merged" }, "detailsWidth": self.details_w, "detailsSplit": self.details_split, "columns": columns, "wipRatio": self.wip_ratio, "wipCollapsed": self.wip_closed, "syncButton": self.sync_mode.key() })
    }

    /// A repository's columns, as they were left.
    pub fn restore_columns(&self, tab: &mut Tab) {
        let v = &self.saved_columns[&tab.worktree];
        if v.is_null() {
            return;
        }
        let d = ColumnPrefs::default();
        let num = |k: &str, or: f32| v[k].as_f64().map_or(or, |n| n as f32);
        tab.column_prefs = ColumnPrefs { labels: num("labels", d.labels), graph: v["graph"].as_f64().map(|n| n as f32), author: num("author", d.author), date: num("date", d.date), sha: num("sha", d.sha), message: v["message"].as_f64().map(|n| n as f32) };
        for (c, _) in HIDEABLE {
            if v["hidden"].as_array().is_some_and(|h| h.iter().any(|x| x.as_str() == Some(col_key(c)))) {
                tab.hidden_columns.insert(c);
            }
        }
    }

    /// At a drag's end; off the UI thread's way, it's one small write.
    pub fn save_layout(&self) {
        let Some(path) = file() else { return };
        let text = self.layout_json().to_string();
        self.rt.spawn(async move {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            std::fs::write(path, text).ok();
        });
    }
}
