//! Download history: the last N completed/failed downloads,
//! persisted as JSON next to the settings. Powers the tray menu's
//! recent list, the Downloads window, and "open folder" actions.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_HISTORY: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unix seconds when the download finished (or failed).
    pub finished_at_unix: i64,
    pub url: String,
    pub title: String,
    pub status: HistoryStatus,
    /// Final media file, when known.
    pub file: Option<PathBuf>,
    /// Human-readable failure reason, when failed.
    pub error: Option<String>,
    pub preset_label: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    /// Newest first.
    pub entries: Vec<HistoryEntry>,
}

impl History {
    pub fn load() -> Self {
        let path = crate::settings::Settings::history_path();
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = crate::settings::config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = dir.join("history.json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, crate::settings::Settings::history_path())
    }

    pub fn push(&mut self, entry: HistoryEntry) {
        self.entries.insert(0, entry);
        self.entries.truncate(MAX_HISTORY);
    }

    pub fn recent(&self, n: usize) -> &[HistoryEntry] {
        &self.entries[..n.min(self.entries.len())]
    }
}

/// Current unix time in seconds (kept here so history entries are
/// trivially stampable from anywhere without a datetime crate).
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str) -> HistoryEntry {
        HistoryEntry {
            finished_at_unix: 1,
            url: "https://youtu.be/x".into(),
            title: title.into(),
            status: HistoryStatus::Completed,
            file: None,
            error: None,
            preset_label: "Best quality".into(),
        }
    }

    #[test]
    fn push_keeps_newest_first_and_caps() {
        let mut h = History::default();
        for i in 0..(MAX_HISTORY + 5) {
            h.push(entry(&format!("t{i}")));
        }
        assert_eq!(h.entries.len(), MAX_HISTORY);
        assert_eq!(h.entries[0].title, format!("t{}", MAX_HISTORY + 4));
    }

    #[test]
    fn json_round_trip() {
        let mut h = History::default();
        h.push(entry("a"));
        let text = serde_json::to_string(&h).unwrap();
        let back: History = serde_json::from_str(&text).unwrap();
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].status, HistoryStatus::Completed);
    }
}
