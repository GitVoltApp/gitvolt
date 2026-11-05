//! The sizes a drag sets, remembered: the sidebar's width and panels, the details panel's width and
//! the graph's columns. The web UI keeps them in the profile; until this build has profiles they're a
//! small file in its own cache folder.

use crate::app::GitVolt;
use crate::sidebar::SectionKind;
use serde_json::{Value, json};
use std::path::PathBuf;

const KINDS: [SectionKind; 6] = [SectionKind::Local, SectionKind::Remote, SectionKind::Worktrees, SectionKind::Stashes, SectionKind::Tags, SectionKind::Submodules];

fn file() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(gitvolt_core::paths::cache_base()?.join("gitvolt-gpui").join("layout.json"))
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
        for kind in KINDS {
            if let Some(h) = v["sidebarPanels"][kind.key()].as_f64() {
                self.side_weights.insert(kind, h as f32);
            }
        }
    }

    pub fn layout_json(&self) -> Value {
        let panels: serde_json::Map<String, Value> = self.side_weights.iter().map(|(k, h)| (k.key().to_string(), json!(h))).collect();
        json!({ "sidebarWidth": self.sidebar_w, "sidebarNarrow": self.sidebar_narrow, "sidebarPanels": panels })
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
