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
    /// Return original queue indices matching a case-insensitive path filter.
    /// Filtering never changes playback order and duplicate entries remain distinct.
    pub fn matching_indices(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, path)| {
                (query.is_empty() || path.to_string_lossy().to_lowercase().contains(&query))
                    .then_some(index)
            })
            .collect()
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

/// Results from an explicitly selected local M3U/M3U8 playlist. Entries remain
/// in playlist order; importing never opens or fetches referenced media.
#[derive(Debug)]
pub struct PlaylistImport {
    pub files: Vec<PathBuf>,
    pub skipped: usize,
    pub limit_reached: bool,
}

const MAX_PLAYLIST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PLAYLIST_LINE_BYTES: usize = 32_768;

/// Read a bounded UTF-8 M3U/M3U8 file. Comments/EXTINF are ignored; URL,
/// network-share, nested-playlist and unsupported entries are skipped. Relative
/// paths are resolved against the playlist's directory; existence is checked
/// only when playback is requested. No referenced files are opened here.
pub fn import_playlist(path: &Path, capacity: usize) -> Result<PlaylistImport, String> {
    let extension = path.extension().and_then(|v| v.to_str()).unwrap_or("");
    if !["m3u", "m3u8"]
        .iter()
        .any(|ext| extension.eq_ignore_ascii_case(ext))
    {
        return Err("Choose an M3U or M3U8 playlist".into());
    }
    let path = path
        .canonicalize()
        .map_err(|e| format!("Cannot open playlist: {e}"))?;
    if !path.is_file() {
        return Err("Playlist must be a file".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .map_err(|e| format!("Cannot open playlist: {e}"))?
        .take(MAX_PLAYLIST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read playlist: {e}"))?;
    if bytes.len() as u64 > MAX_PLAYLIST_BYTES {
        return Err("Playlist is too large".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "Playlist must be UTF-8")?;
    let base = path.parent().ok_or("Playlist has no parent directory")?;
    parse_playlist(text, base, capacity.min(MAX_QUEUE))
}

/// Reject URL and network-share references while allowing Windows verbatim
/// local drive paths (returned by `canonicalize`) to round-trip in playlists.
fn unsafe_playlist_reference(value: &str) -> bool {
    if value.contains('\0') || value.contains("://") || value.starts_with("//") {
        return true;
    }
    if let Some(rest) = value.strip_prefix("\\\\?\\") {
        let bytes = rest.as_bytes();
        return !(bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/'));
    }
    value.starts_with("\\\\")
}

fn parse_playlist(text: &str, base: &Path, capacity: usize) -> Result<PlaylistImport, String> {
    let mut result = PlaylistImport {
        files: Vec::new(),
        skipped: 0,
        limit_reached: false,
    };
    for line in text.trim_start_matches('\u{feff}').lines() {
        if line.len() > MAX_PLAYLIST_LINE_BYTES {
            return Err("Playlist contains an oversized line".into());
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if unsafe_playlist_reference(line) {
            result.skipped += 1;
            continue;
        }
        let candidate = Path::new(line);
        if !supported_path(candidate)
            || (!candidate.is_absolute()
                && (candidate.has_root()
                    || candidate
                        .components()
                        .any(|c| matches!(c, std::path::Component::Prefix(_)))))
        {
            result.skipped += 1;
            continue;
        }
        if result.files.len() >= capacity {
            result.limit_reached = true;
            break;
        }
        result.files.push(if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            base.join(candidate)
        });
    }
    Ok(result)
}

/// Export one UTF-8 M3U playlist to a new file; never overwrite an existing
/// destination. Path conversion is validated before the output is created.
/// Files that no longer exist are retained, but URLs, network shares and paths
/// that cannot be represented safely as a single line reject the entire export.
/// An I/O failure after creation can leave a partial file.
pub fn export_playlist(path: &Path, queue: &PlayQueue) -> Result<(), String> {
    let extension = path.extension().and_then(|v| v.to_str()).unwrap_or("");
    if !["m3u", "m3u8"]
        .iter()
        .any(|ext| extension.eq_ignore_ascii_case(ext))
    {
        return Err("Choose an M3U or M3U8 output filename".into());
    }
    let current =
        std::env::current_dir().map_err(|e| format!("Cannot resolve relative paths: {e}"))?;
    let mut text = String::from("#EXTM3U\n");
    for entry in queue.items() {
        let raw = entry.to_str().ok_or("Playlist paths must be UTF-8")?;
        if unsafe_playlist_reference(raw) || raw.contains(['\r', '\n']) || !supported_path(entry) {
            return Err("Queue contains a URL, share, unsupported or multiline path".into());
        }
        let absolute = if entry.is_absolute() {
            entry.clone()
        } else {
            current.join(entry)
        };
        let line = absolute.to_str().ok_or("Playlist paths must be UTF-8")?;
        if !absolute.is_absolute()
            || unsafe_playlist_reference(line)
            || line.contains(['\r', '\n'])
            || line.len() > MAX_PLAYLIST_LINE_BYTES
        {
            return Err("Queue contains a path that cannot be exported safely".into());
        }
        text.push_str(line);
        text.push('\n');
        if text.len() as u64 > MAX_PLAYLIST_BYTES {
            return Err("Playlist is too large".into());
        }
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot create playlist: {e}"))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("Cannot write playlist; a partial file may remain: {e}"))
}

/// A bounded scan result. `limit_reached` means a scan limit was reached;
/// `skipped` counts unreadable entries and intentionally ignored links.
#[derive(Debug)]
pub struct FolderScan {
    pub files: Vec<PathBuf>,
    pub skipped: usize,
    pub limit_reached: bool,
}

/// Scan a selected folder off-thread. No child symlink or Windows reparse
/// point is followed. Traversal, results, depth and cancellation are bounded.
pub fn scan_folder(
    folder: &Path,
    capacity: usize,
    token: &CancellationToken,
) -> Result<FolderScan, String> {
    scan_folder_with_limits(folder, capacity.min(MAX_QUEUE), 10_000, 512, 16, token)
}

fn scan_folder_with_limits(
    folder: &Path,
    capacity: usize,
    max_entries: usize,
    max_dirs: usize,
    max_depth: usize,
    token: &CancellationToken,
) -> Result<FolderScan, String> {
    if token.is_cancelled() {
        return Err("Folder scan cancelled".into());
    }
    let root = folder
        .canonicalize()
        .map_err(|e| format!("Cannot open folder: {e}"))?;
    if !root.is_dir() {
        return Err("Choose a directory".into());
    }
    let mut result = FolderScan {
        files: Vec::new(),
        skipped: 0,
        limit_reached: false,
    };
    if capacity == 0 {
        result.limit_reached = true;
        return Ok(result);
    }
    let mut pending = vec![(root, 0usize)];
    let mut entries_seen = 0usize;
    let mut dirs_seen = 0usize;
    while let Some((directory, depth)) = pending.pop() {
        if token.is_cancelled() {
            return Err("Folder scan cancelled".into());
        }
        if dirs_seen >= max_dirs {
            result.limit_reached = true;
            break;
        }
        dirs_seen += 1;
        let reader = match std::fs::read_dir(&directory) {
            Ok(reader) => reader,
            Err(e) if depth == 0 => return Err(format!("Cannot read folder: {e}")),
            Err(_) => {
                result.skipped += 1;
                continue;
            }
        };
        let mut paths = Vec::new();
        for entry in reader {
            if token.is_cancelled() {
                return Err("Folder scan cancelled".into());
            }
            if entries_seen >= max_entries {
                result.limit_reached = true;
                break;
            }
            entries_seen += 1;
            match entry {
                Ok(entry) => paths.push(entry.path()),
                Err(_) => result.skipped += 1,
            }
        }
        paths.sort();
        for path in paths {
            if token.is_cancelled() {
                return Err("Folder scan cancelled".into());
            }
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    result.skipped += 1;
                    continue;
                }
            };
            if is_link_or_reparse(&metadata) {
                result.skipped += 1;
                continue;
            }
            if metadata.is_dir() {
                if depth >= max_depth || pending.len() >= max_dirs {
                    result.limit_reached = true;
                } else {
                    pending.push((path, depth + 1));
                }
            } else if metadata.is_file() && supported_path(&path) {
                result.files.push(path);
                if result.files.len() >= capacity {
                    result.limit_reached = true;
                    break;
                }
            }
        }
        if entries_seen >= max_entries {
            result.limit_reached = true;
            break;
        }
        if result.files.len() >= capacity {
            break;
        }
    }
    if token.is_cancelled() {
        return Err("Folder scan cancelled".into());
    }
    result.files.sort();
    Ok(result)
}

fn is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Preserve the selected track after removing an entry before it. If the
/// selected entry itself is removed, select its successor (or the new last).
pub fn selection_after_remove(selected: usize, removed: usize, remaining: usize) -> usize {
    if remaining == 0 {
        return 0;
    }
    selected
        .saturating_sub(usize::from(removed < selected))
        .min(remaining - 1)
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
    write_atomic(path, &bytes)
}

/// Player preferences are saved only when explicitly requested; loading them
/// does not launch a process or start playback.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerOptions {
    version: u8,
    pub ffplay_path: String,
    pub volume: u8,
    pub repeat: bool,
}
impl PlayerOptions {
    pub fn new(ffplay_path: String, volume: u8, repeat: bool) -> Self {
        Self {
            version: 1,
            ffplay_path,
            volume,
            repeat,
        }
    }
    fn validate(&self) -> io::Result<()> {
        if self.version != 1
            || self.volume > 100
            || self.ffplay_path.len() > 32_768
            || self.ffplay_path.contains('\0')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid or unsupported player options",
            ));
        }
        Ok(())
    }
}
const MAX_PLAYER_OPTIONS_BYTES: u64 = 128 * 1024;

pub fn player_options_path() -> PathBuf {
    crate::settings::config_dir().join("player-options.json")
}

/// Save options with write-then-rename, preserving the previous file if validation fails.
pub fn save_options(path: &Path, options: &PlayerOptions) -> io::Result<()> {
    options.validate()?;
    let bytes = serde_json::to_vec_pretty(options)?;
    if bytes.len() as u64 > MAX_PLAYER_OPTIONS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Player options are too large",
        ));
    }
    write_atomic(path, &bytes)
}

/// A malformed, unsupported or oversized file is rejected without changing state.
pub fn load_options(path: &Path) -> io::Result<PlayerOptions> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_PLAYER_OPTIONS_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PLAYER_OPTIONS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Player options are too large",
        ));
    }
    let options: PlayerOptions = serde_json::from_slice(&bytes)?;
    options.validate()?;
    Ok(options)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
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
    fn local_playlist_import_is_bounded_and_never_accepts_urls() {
        let dir = std::env::temp_dir().join(format!("yttray-playlist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let path = dir.join("list.m3u8");
        std::fs::write(&path, "\u{feff}#EXTM3U\n#EXTINF:12,Title\na song.MP3\nhttps://example.test/audio.mp3\nfile:///remote.mp4\n//server/share/a.mp3\nother.m3u\nfolder/second.flac\n").unwrap();
        let result = import_playlist(&path, 10).unwrap();
        assert_eq!(
            result.files,
            [dir.join("a song.MP3"), dir.join("folder/second.flac")]
        );
        assert_eq!(result.skipped, 4);
        assert!(!result.limit_reached);
        let result = import_playlist(&path, 1).unwrap();
        assert_eq!(result.files, [dir.join("a song.MP3")]);
        assert!(result.limit_reached);
        assert!(import_playlist(&dir.join("list.txt"), 10).is_err());
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(import_playlist(&path, 10).is_err());
        std::fs::write(
            &path,
            format!("{}.mp3", "x".repeat(MAX_PLAYLIST_LINE_BYTES)),
        )
        .unwrap();
        assert!(import_playlist(&path, 10).is_err());
        std::fs::write(&path, vec![b'x'; MAX_PLAYLIST_BYTES as usize + 1]).unwrap();
        assert!(import_playlist(&path, 10).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn playlist_export_round_trips_and_never_overwrites() {
        let dir =
            std::env::temp_dir().join(format!("yttray-playlist-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let mut queue = PlayQueue::default();
        queue.add([
            dir.join("music & one.mp3"),
            dir.join("missing.flac"),
            dir.join("music & one.mp3"),
        ]);
        let out = dir.join("queue.m3u8");
        export_playlist(&out, &queue).unwrap();
        assert_eq!(
            import_playlist(&out, MAX_QUEUE).unwrap().files,
            queue.items()
        );
        let bytes = std::fs::read(&out).unwrap();
        assert!(export_playlist(&out, &queue).is_err());
        assert_eq!(std::fs::read(&out).unwrap(), bytes);
        let mut invalid = PlayQueue::default();
        invalid.add([PathBuf::from("https://example.test/song.mp3")]);
        assert!(export_playlist(&dir.join("bad.m3u"), &invalid).is_err());
        assert!(!dir.join("bad.m3u").exists());
        let mut multiline = PlayQueue::default();
        multiline.add([PathBuf::from("line\nbreak.mp3")]);
        assert!(export_playlist(&dir.join("bad.m3u"), &multiline).is_err());
        assert!(!dir.join("bad.m3u").exists());
        assert!(!unsafe_playlist_reference(r"\\?\C:\Music\song.mp3"));
        assert!(unsafe_playlist_reference(r"\\?\UNC\host\share\song.mp3"));
        assert!(unsafe_playlist_reference(r"\\host\share\song.mp3"));
        assert!(export_playlist(&dir.join("out.txt"), &queue).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn player_options_round_trip_and_reject_invalid_files() {
        let dir =
            std::env::temp_dir().join(format!("yttray-player-options-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("player-options.json");
        let options = PlayerOptions::new("C:/Tools/ffplay.exe".into(), 35, true);
        save_options(&path, &options).unwrap();
        assert_eq!(load_options(&path).unwrap(), options);
        let original = std::fs::read(&path).unwrap();
        assert!(save_options(&path, &PlayerOptions::new("bad\0path".into(), 100, false)).is_err());
        assert!(save_options(&path, &PlayerOptions::new(String::new(), 101, false)).is_err());
        assert!(save_options(&path, &PlayerOptions::new("x".repeat(32769), 50, false)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        for text in [
            r#"{"version":2,"ffplay_path":"","volume":50,"repeat":false}"#,
            r#"{"version":1,"ffplay_path":"","volume":101,"repeat":false}"#,
            r#"{"version":1,"ffplay_path":"","volume":50,"repeat":false,"extra":true}"#,
            "broken",
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(load_options(&path).is_err());
        }
        std::fs::write(&path, vec![b'x'; MAX_PLAYER_OPTIONS_BYTES as usize + 1]).unwrap();
        assert!(load_options(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn folder_scan_is_bounded_sorted_and_cancellable() {
        let dir = std::env::temp_dir().join(format!("yttray-player-scan-{}", std::process::id()));
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let dir = dir.canonicalize().unwrap();
        let nested = dir.join("nested");
        for path in [
            dir.join("B.mp3"),
            dir.join("a.MP4"),
            dir.join("ignore.txt"),
            nested.join("c.flac"),
        ] {
            std::fs::write(path, b"fixture").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&nested, dir.join("link")).unwrap();
        let token = CancellationToken::default();
        let result = scan_folder(&dir, 10, &token).unwrap();
        assert_eq!(
            result.files,
            [dir.join("B.mp3"), dir.join("a.MP4"), nested.join("c.flac")]
        );
        #[cfg(unix)]
        assert_eq!(result.skipped, 1);
        assert!(!result.limit_reached);
        let capped = scan_folder(&dir, 1, &token).unwrap();
        assert_eq!(capped.files.len(), 1);
        assert!(capped.limit_reached);
        let shallow = scan_folder_with_limits(&dir, 10, 100, 10, 0, &token).unwrap();
        assert_eq!(shallow.files.len(), 2);
        assert!(shallow.limit_reached);
        let capped_entries = scan_folder_with_limits(&dir, 10, 1, 10, 10, &token).unwrap();
        assert!(capped_entries.limit_reached);
        token.cancel();
        assert!(scan_folder(&dir, 10, &token)
            .unwrap_err()
            .contains("cancelled"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn filtering_keeps_original_indices_and_selection_tracks_removal() {
        let mut queue = PlayQueue::default();
        queue.add([
            "Music/Álbum/First.MP3".into(),
            "Other/second.mp4".into(),
            "Music/Álbum/First.MP3".into(),
        ]);
        assert_eq!(queue.matching_indices("  áLBUM  "), [0, 2]);
        assert_eq!(queue.matching_indices("second"), [1]);
        assert!(queue.matching_indices("missing").is_empty());
        assert_eq!(queue.matching_indices(" "), [0, 1, 2]);
        assert_eq!(queue.next(0, false), Some(1));
        assert!(queue.move_down(0));
        assert_eq!(queue.matching_indices("ÁLBUM"), [1, 2]);
        assert_eq!(selection_after_remove(2, 0, 2), 1);
        assert_eq!(selection_after_remove(1, 1, 2), 1);
        assert_eq!(selection_after_remove(2, 2, 2), 1);
        assert_eq!(selection_after_remove(0, 0, 0), 0);
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
