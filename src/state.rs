use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::conf::state_dir;

/// Copy history, persisted at ~/.local/state/mdrv-em/recents.json.
/// Stored as catalog cp keys ("1F600", "1F44A-1F3FB"), most recent first.
#[derive(Deserialize, Serialize, Default, Clone)]
pub struct Recents {
    pub items: Vec<String>,
}

impl Recents {
    pub fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn push(&mut self, cp: &str) {
        self.items.retain(|c| c != cp);
        self.items.insert(0, cp.to_string());
        self.items.truncate(24);
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(&path, text);
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(state_dir()?.join("recents.json"))
}
