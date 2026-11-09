//! What the app remembers between runs: where you were, and what you've ignored.

use crate::rules;
use crate::window_state::Placement;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub folder: Option<PathBuf>,
    pub tabs: Vec<PathBuf>,
    pub active: Option<usize>,
    /// Comments marked Ignore, by file then hunk key.
    pub ignored: HashMap<PathBuf, HashSet<u64>>,
    pub window: Option<Placement>,
    /// Limit cleaning to comments on lines changed since the last commit.
    pub only_changed: bool,
    /// Off by default: updates are found and downloaded automatically.
    pub manual_updates: bool,
    /// (version, release notes), to show once that version has started.
    pub whats_new: Option<(String, String)>,
}

fn path() -> Option<PathBuf> {
    rules::config_file("session.json")
}

pub fn load() -> Session {
    path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(session: &Session) -> Result<(), String> {
    let path = path().ok_or("no settings folder")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string(session).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("Couldn't save your session: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_keys_survive_the_round_trip() {
        let session = Session {
            folder: Some("C:/work".into()),
            tabs: vec!["C:/work/a.ts".into()],
            active: Some(0),
            ignored: HashMap::from([("C:/work/a.ts".into(), HashSet::from([u64::MAX]))]),
            ..Session::default()
        };
        let text = serde_json::to_string(&session).unwrap();
        let back: Session = serde_json::from_str(&text).unwrap();
        assert_eq!(back.ignored[&PathBuf::from("C:/work/a.ts")], HashSet::from([u64::MAX]));
        assert_eq!(back.active, Some(0));
    }

    #[test]
    fn a_damaged_or_older_file_falls_back_to_defaults() {
        let back: Session = serde_json::from_str("{}").unwrap();
        assert!(back.folder.is_none() && back.tabs.is_empty());
    }
}
