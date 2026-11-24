//! Local playback queue and shell-free invocation of a separately installed ffplay.
//! Media and players are owned by the application; no online playback is supported.
use crate::process::{self, CancellationToken, ProcessLimits, ProcessSpec};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MAX_QUEUE: usize = 500;

/// Supported local media extensions. Playlist and URL inputs are deliberately excluded.
pub fn supported_path(path: &Path) -> bool {
    path.extension()
        .and_then(|v| v.to_str())
        .is_some_and(|ext| {
            [
                "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "wma", "mp4", "m4v", "mov",
                "mkv", "webm", "avi", "mpg", "mpeg", "wmv",
            ]
            .iter()
            .any(|supported| ext.eq_ignore_ascii_case(supported))
        })
}

#[derive(Default, Debug, Clone)]
pub struct PlayQueue {
    items: Vec<PathBuf>,
}
impl PlayQueue {
    pub fn items(&self) -> &[PathBuf] {
        &self.items
    }
    /// Adds supported entries, up to the queue limit. File access is checked in the worker.
    pub fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> usize {
        let mut added = 0;
        for path in paths {
            if self.items.len() >= MAX_QUEUE {
                break;
            }
            if supported_path(&path) {
                self.items.push(path);
                added += 1;
            }
        }
        added
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.items.len() {
            return false;
        }
        self.items.remove(index);
        true
    }
    pub fn move_up(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.items.len() {
            return false;
        }
        self.items.swap(index, index - 1);
        true
    }
    pub fn move_down(&mut self, index: usize) -> bool {
        if index >= self.items.len().saturating_sub(1) {
            return false;
        }
        self.items.swap(index, index + 1);
        true
    }
    pub fn next(&self, index: usize, repeat: bool) -> Option<usize> {
        if index >= self.items.len() {
            return None;
        }
        if index + 1 < self.items.len() {
            Some(index + 1)
        } else if repeat && !self.items.is_empty() {
            Some(0)
        } else {
            None
        }
    }
}

/// Maximum on-disk playlist size; file paths may contain private information.
pub const MAX_SAVED_QUEUE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedQueue {
    version: u8,
    items: Vec<PathBuf>,
}

/// Explicitly saved queue in the application's normal or portable data directory.
pub fn saved_queue_path() -> PathBuf {
    crate::settings::config_dir().join("player-queue.json")
}

/// Save a snapshot atomically. Relative paths are resolved now, without
/// requiring the files to exist. It never changes the original media files.
pub fn save_queue(path: &Path, queue: &PlayQueue) -> io::Result<()> {
    let current = std::env::current_dir()?;
    let items: Vec<PathBuf> = queue
        .items()
        .iter()
        .map(|item| {
            if item.is_absolute() {
                item.clone()
            } else {
                current.join(item)
            }
        })
        .collect();
    validate_saved_items(&items)?;
    let bytes = serde_json::to_vec_pretty(&SavedQueue { version: 1, items })?;
    if bytes.len() as u64 > MAX_SAVED_QUEUE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Saved queue is too large",
        ));
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

/// Read and strictly validate a saved queue without touching the live queue.
/// Missing media is retained; playback reports inaccessible files when selected.
pub fn load_queue(path: &Path) -> io::Result<PlayQueue> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_SAVED_QUEUE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SAVED_QUEUE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Saved queue is too large",
        ));
    }
    let saved: SavedQueue = serde_json::from_slice(&bytes)?;
    if saved.version != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unsupported saved queue version",
        ));
    }
    validate_saved_items(&saved.items)?;
    Ok(PlayQueue { items: saved.items })
}

fn validate_saved_items(items: &[PathBuf]) -> io::Result<()> {
    if items.len() > MAX_QUEUE
        || items.iter().any(|path| {
            !path.is_absolute()
                || !supported_path(path)
                || path.to_str().is_none_or(|s| s.contains('\0'))
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Saved queue contains invalid or unsupported paths",
        ));
    }
    Ok(())
}

/// Decide the next queue action after a track exits. An explicit skip wins
/// over cancellation, while an error never silently advances or repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    Start(usize),
    Stopped,
    Finished,
    Failed,
}
pub fn completion(
    queue: &PlayQueue,
    index: usize,
    repeat: bool,
    stopping: bool,
    pending: Option<usize>,
    success: bool,
) -> Completion {
    if let Some(next) = pending {
        return if next < queue.items().len() {
            Completion::Start(next)
        } else {
            Completion::Stopped
        };
    }
    if stopping {
        return Completion::Stopped;
    }
    if !success {
        return Completion::Failed;
    }
    match queue.next(index, repeat) {
        Some(next) => Completion::Start(next),
        None => Completion::Finished,
    }
}

/// Locate ffplay beside the selected ffmpeg or on PATH. An explicitly selected
/// executable takes priority and does not silently fall back if missing.
pub fn locate_ffplay(explicit: Option<&Path>, ffmpeg: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return path.is_file().then(|| path.to_path_buf());
    }
    let name = if cfg!(windows) {
        "ffplay.exe"
    } else {
        "ffplay"
    };
    ffmpeg
        .and_then(Path::parent)
        .map(|p| p.join(name))
        .filter(|p| p.is_file())
        .or_else(|| crate::ytdlp::find_on_path(name))
}

/// Build a local-file-only invocation. ffplay owns its own playback window,
/// where pause, seek and volume keys are handled by ffplay itself.
pub fn build_playback(ffplay: &Path, input: &Path, volume: u8) -> Result<ProcessSpec, String> {
    if volume > 100 {
        return Err("Volume must be between 0 and 100".into());
    }
    if !supported_path(input) {
        return Err("Unsupported media type or playlist".into());
    }
    let input = input
        .canonicalize()
        .map_err(|e| format!("Cannot open media: {e}"))?;
    if !input.is_file() {
        return Err("Media must be a local file".into());
    }
    Ok(ProcessSpec::new(ffplay)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostats",
            "-autoexit",
            "-protocol_whitelist",
            "file,crypto,data",
            "-window_title",
            "ytTrayDL Playback",
            "-volume",
        ])
        .arg(volume.to_string())
        .arg("-i")
        .arg(input))
}

/// Run one track under the owned-process supervisor. Cancel kills only this track.
pub fn play(spec: &ProcessSpec, token: &CancellationToken) -> Result<(), String> {
    if token.is_cancelled() {
        return Err("Playback cancelled".into());
    }
    let limits = ProcessLimits {
        timeout: Duration::from_secs(24 * 60 * 60),
        stdout_bytes: 8192,
        stderr_bytes: 32 * 1024,
        line_bytes: 8192,
    };
    process::OwnedProcess::spawn(spec)
        .and_then(|p| p.stream(limits, token, |_, _| {}))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_is_bounded_and_editable() {
        let mut queue = PlayQueue::default();
        assert_eq!(
            queue.add(["a.mp3".into(), "bad.m3u".into(), "B.MP4".into()]),
            2
        );
        assert!(queue.move_down(0));
        assert_eq!(queue.items()[0], Path::new("B.MP4"));
        assert!(queue.move_up(1));
        assert!(!queue.move_up(0));
        assert_eq!(queue.next(1, false), None);
        assert_eq!(queue.next(1, true), Some(0));
        assert_eq!(queue.next(2, true), None);
        assert!(queue.remove(0));
        assert!(!queue.remove(2));
        assert_eq!(
            queue.add((0..600).map(|i| PathBuf::from(format!("{i}.mp3")))),
            MAX_QUEUE - 1
        );
        assert_eq!(queue.items().len(), MAX_QUEUE);
        queue.clear();
        assert_eq!(queue.next(0, true), None);
    }
    #[test]
    fn saved_queue_roundtrip_replacement_and_invalid_data() {
        let dir = std::env::temp_dir().join(format!("yttray-player-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("player-queue.json");
        let mut queue = PlayQueue::default();
        queue.add([dir.join("missing song.mp3"), dir.join("video.mp4")]);
        save_queue(&path, &queue).unwrap();
        assert_eq!(load_queue(&path).unwrap().items(), queue.items());
        queue.remove(0);
        save_queue(&path, &queue).unwrap();
        assert_eq!(load_queue(&path).unwrap().items(), queue.items());
        let original = std::fs::read(&path).unwrap();
        let oversized = PlayQueue {
            items: vec![dir.join(format!(
                "{}.mp3",
                "x".repeat(MAX_SAVED_QUEUE_BYTES as usize)
            ))],
        };
        assert!(save_queue(&path, &oversized).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        for content in [
            r#"{"version":2,"items":[]}"#,
            r#"{"version":1,"items":["relative.mp3"]}"#,
            r#"{"version":1,"items":[],"extra":true}"#,
            "not json",
        ] {
            std::fs::write(&path, content).unwrap();
            assert!(load_queue(&path).is_err());
        }
        std::fs::write(&path, vec![b'x'; MAX_SAVED_QUEUE_BYTES as usize + 1]).unwrap();
        assert!(load_queue(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn completion_handles_repeat_skip_stop_and_failure() {
        let mut queue = PlayQueue::default();
        queue.add(["a.mp3".into(), "b.mp4".into()]);
        assert_eq!(
            completion(&queue, 0, false, false, None, true),
            Completion::Start(1)
        );
        assert_eq!(
            completion(&queue, 1, false, false, None, true),
            Completion::Finished
        );
        assert_eq!(
            completion(&queue, 1, true, false, None, true),
            Completion::Start(0)
        );
        assert_eq!(
            completion(&queue, 0, true, true, None, false),
            Completion::Stopped
        );
        assert_eq!(
            completion(&queue, 0, false, true, Some(1), false),
            Completion::Start(1)
        );
        assert_eq!(
            completion(&queue, 0, false, true, Some(99), false),
            Completion::Stopped
        );
        assert_eq!(
            completion(&queue, 0, true, false, None, false),
            Completion::Failed
        );
    }

    #[test]
    fn playback_is_local_argv_and_volume_is_bounded() {
        let dir = std::env::temp_dir().join(format!("yttray-player-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song & test.mp3");
        std::fs::write(&path, b"fixture").unwrap();
        let spec = build_playback(Path::new("ffplay"), &path, 75).unwrap();
        assert!(spec
            .args
            .windows(2)
            .any(|pair| pair[0] == "-volume" && pair[1] == "75"));
        assert!(spec
            .args
            .windows(2)
            .any(|pair| pair[0] == "-protocol_whitelist" && pair[1] == "file,crypto,data"));
        assert_eq!(
            spec.args.last().unwrap(),
            path.canonicalize().unwrap().as_os_str()
        );
        assert!(build_playback(Path::new("ffplay"), &path, 101).is_err());
        assert!(build_playback(
            Path::new("ffplay"),
            Path::new("https://example.test/a.mp3"),
            0
        )
        .is_err());
        assert!(build_playback(Path::new("ffplay"), &dir.join("list.m3u"), 0).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
