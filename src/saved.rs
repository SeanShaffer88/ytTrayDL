//! Small persisted side-data: the "save for later" link list
//! and the channel watcher's bookkeeping. Both live as JSON in
//! the config dir next to settings/history, load tolerantly
//! (any error → empty) and save best-effort.

use crate::settings::config_dir;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A link the user parked from the popup instead of downloading
/// right away.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedLink {
    pub url: String,
    pub title: String,
    pub saved_at_unix: i64,
}

fn saved_path() -> std::path::PathBuf {
    config_dir().join("saved.json")
}

pub fn load_saved() -> Vec<SavedLink> {
    let Ok(text) = std::fs::read_to_string(saved_path()) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_saved(links: &[SavedLink]) -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let text = serde_json::to_string_pretty(links)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = dir.join("saved.json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, saved_path())
}

/// Channel-watcher bookkeeping: which entry ids each subscribed
/// URL has already shown us, and when the last poll ran. A URL
/// absent from `seen` has never been polled — its first poll
/// only baselines (records current entries, downloads nothing),
/// so adding a big channel never dumps its back-catalogue.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchData {
    pub seen: HashMap<String, Vec<String>>,
    pub last_check_unix: u64,
}

/// How many seen ids are kept per subscription (enough to cover
/// the probe window many times over; older ids age out).
pub const SEEN_CAP: usize = 300;

impl WatchData {
    pub fn load() -> Self {
        let Ok(text) = std::fs::read_to_string(config_dir().join("watch.json")) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = dir.join("watch.json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, dir.join("watch.json"))
    }

    /// Record a poll of `url` that returned `ids` (newest first,
    /// as probed). Returns the ids that are NEW — never returned
    /// for a first-ever poll (baseline: everything is recorded,
    /// nothing reported).
    pub fn record_poll(&mut self, url: &str, ids: &[String]) -> Vec<String> {
        let Some(known) = self.seen.get_mut(url) else {
            self.seen.insert(url.to_string(), ids.to_vec());
            return Vec::new();
        };
        let fresh: Vec<String> = ids
            .iter()
            .filter(|id| !known.contains(id))
            .cloned()
            .collect();
        for id in ids {
            if !known.contains(id) {
                known.push(id.clone());
            }
        }
        if known.len() > SEEN_CAP {
            let drop = known.len() - SEEN_CAP;
            known.drain(..drop);
        }
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_poll_baselines_and_second_reports_new() {
        let mut w = WatchData::default();
        let first: Vec<String> = vec!["a".into(), "b".into()];
        assert!(w.record_poll("chan", &first).is_empty());
        let second: Vec<String> = vec!["c".into(), "a".into(), "b".into()];
        assert_eq!(w.record_poll("chan", &second), vec!["c".to_string()]);
        // Re-polling the same set reports nothing.
        assert!(w.record_poll("chan", &second).is_empty());
    }

    #[test]
    fn seen_cap_trims_oldest() {
        let mut w = WatchData::default();
        let ids: Vec<String> = (0..SEEN_CAP + 10).map(|i| i.to_string()).collect();
        w.record_poll("chan", &ids);
        assert_eq!(w.seen["chan"].len(), SEEN_CAP + 10); // baseline keeps all
        let more: Vec<String> = (0..5).map(|i| format!("n{i}")).collect();
        w.record_poll("chan", &more);
        assert_eq!(w.seen["chan"].len(), SEEN_CAP);
    }
}
