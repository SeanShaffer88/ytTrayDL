//! ytTrayDL library: all logic that does not draw pixels.
//!
//! Modules:
//! - [`settings`] — persisted user configuration (JSON, versioned).
//! - [`url_detect`] — URL extraction from clipboard text, host
//!   parsing, the built-in known-sites list.
//! - [`rules`] — the decision engine: Prompt / AutoDownload / Ignore
//!   for a detected URL, from the user's domain rules and defaults.
//! - [`dedupe`] — short-window duplicate suppression.
//! - [`ytdlp`] — locating yt-dlp/ffmpeg, building command lines,
//!   parsing progress output and info JSON.
//! - [`download`] — the download job queue/manager.
//! - [`history`] — persisted download history.
//! - [`platform`] — Win32 clipboard listener, single-instance,
//!   autostart, shell helpers (with non-Windows fallbacks).

pub mod process;
pub mod window_geometry;

/// App-owned workers share cancellation and are joined during shutdown.
#[derive(Default)]
pub struct BackgroundTasks {
    token: process::CancellationToken,
    workers: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
}
impl BackgroundTasks {
    pub fn spawn(&self, work: impl FnOnce(process::CancellationToken) + Send + 'static) {
        let token = self.token.clone();
        let mut workers = self.workers.lock().unwrap();
        if self.token.is_cancelled() {
            return;
        }
        let mut pending = Vec::new();
        for worker in workers.drain(..) {
            if worker.is_finished() {
                if worker.join().is_err() {
                    diag_log("background worker panicked");
                }
            } else {
                pending.push(worker);
            }
        }
        pending.push(std::thread::spawn(move || work(token)));
        *workers = pending;
    }
    pub fn shutdown(&self) {
        self.token.cancel();
        for worker in self.workers.lock().unwrap().drain(..) {
            if worker.join().is_err() {
                diag_log("background shutdown: worker panicked");
            }
        }
    }
}
impl Drop for BackgroundTasks {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn capture(
    spec: &process::ProcessSpec,
    seconds: u64,
    bytes: usize,
    token: &process::CancellationToken,
) -> Result<process::ProcessOutput, process::ProcessError> {
    process::run_capture(
        spec,
        process::ProcessLimits {
            timeout: std::time::Duration::from_secs(seconds),
            stdout_bytes: bytes,
            line_bytes: bytes,
            ..Default::default()
        },
        token,
    )
}

pub fn fetch_version(bin: &std::path::Path, token: &process::CancellationToken) -> Option<String> {
    let output = capture(
        &process::ProcessSpec::new(bin).arg("--version"),
        10,
        65536,
        token,
    )
    .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Append one line to the local diagnostic log
/// (`<config dir>/yttraydl.log`). A tray app has no console, so
/// startup milestones, panics and fatal errors land here — local
/// only, never sent anywhere. Best-effort: logging never panics
/// and never blocks the app.
pub fn diag_log(message: &str) {
    use std::io::Write;
    let dir = settings::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let line = format!("[{}] {message}\n", history::now_unix());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("yttraydl.log"))
    {
        let _ = f.write_all(line.as_bytes());
    }
}

pub mod dedupe;
pub mod download;
pub mod history;
pub mod hooks;
pub mod platform;
pub mod rules;
pub mod saved;
pub mod settings;
pub mod url_detect;
pub mod ytdlp;

/// Fetch video info with yt-dlp (blocking; call on a worker thread).
/// Returns `None` on any failure — the caller falls back to showing
/// the bare URL, so a slow or unsupported site never blocks a popup.
pub fn fetch_info(
    bin: &std::path::Path,
    settings: &settings::Settings,
    url: &str,
) -> Option<ytdlp::VideoInfo> {
    fetch_info_with_cancel(bin, settings, url, &process::CancellationToken::default())
}

pub fn fetch_info_with_cancel(
    bin: &std::path::Path,
    settings: &settings::Settings,
    url: &str,
    token: &process::CancellationToken,
) -> Option<ytdlp::VideoInfo> {
    let args = ytdlp::build_info_args(settings, url);
    let output = capture(
        &process::ProcessSpec::new(bin).args(&args),
        30,
        8 * 1024 * 1024,
        token,
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // --dump-single-json prints exactly one JSON object; take the
    // last non-empty line defensively in case of stray warnings.
    let json_line = text
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))?;
    ytdlp::parse_info_json(json_line).ok()
}

/// A command-line tool Windows ships in System32 (curl.exe and
/// tar.exe both arrived with Windows 10 1803) — resolved to the
/// full path when present, else the bare name for PATH lookup.
fn bundled_tool(name: &str) -> std::path::PathBuf {
    if cfg!(windows) {
        let system32 = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let p = std::path::PathBuf::from(system32)
            .join("System32")
            .join(format!("{name}.exe"));
        if p.is_file() {
            return p;
        }
        return std::path::PathBuf::from(format!("{name}.exe"));
    }
    std::path::PathBuf::from(name)
}

struct TemporaryFiles(Vec<std::path::PathBuf>);
impl Drop for TemporaryFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn install_downloaded(
    target: &std::path::Path,
    temporary: &std::path::Path,
    download: impl FnOnce(&std::path::Path) -> Result<(), String>,
) -> Result<(), String> {
    let _cleanup = TemporaryFiles(vec![temporary.to_path_buf()]);
    download(temporary)?;
    std::fs::rename(temporary, target).map_err(|e| e.to_string())
}

/// Download the official standalone yt-dlp.exe into the managed bin
/// folder using Windows' bundled curl.exe. Blocking; call on a
/// worker thread. Only supported on Windows builds.
pub fn download_managed_ytdlp() -> Result<std::path::PathBuf, String> {
    download_managed_ytdlp_with_cancel(&process::CancellationToken::default())
}

pub fn download_managed_ytdlp_with_cancel(
    token: &process::CancellationToken,
) -> Result<std::path::PathBuf, String> {
    let dir = settings::Settings::managed_bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let target = dir.join(if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    });
    let tmp = dir.join("yt-dlp.download");
    let curl = bundled_tool("curl");
    install_downloaded(&target, &tmp, |temporary| {
        capture(
            &process::ProcessSpec::new(&curl).args([
                "-L",
                "--fail",
                "-o",
                &temporary.to_string_lossy(),
                ytdlp::YTDLP_WINDOWS_URL,
            ]),
            600,
            1048576,
            token,
        )
        .map(|_| ())
        .map_err(|e| format!("download failed: {e}"))
    })?;
    Ok(target)
}

/// The latest official yt-dlp release tag (e.g. "2025.09.26")
/// from the GitHub API, via the bundled curl. Blocking; call on
/// a worker thread. Used by the "Check for yt-dlp updates"
/// button — ytTrayDL itself never phones home unasked.
pub fn fetch_latest_ytdlp_tag() -> Result<String, String> {
    fetch_latest_ytdlp_tag_with_cancel(&process::CancellationToken::default())
}

pub fn fetch_latest_ytdlp_tag_with_cancel(
    token: &process::CancellationToken,
) -> Result<String, String> {
    let curl = bundled_tool("curl");
    let output = capture(
        &process::ProcessSpec::new(&curl).args([
            "-L",
            "--fail",
            "-H",
            "User-Agent: ytTrayDL",
            "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest",
        ]),
        30,
        1048576,
        token,
    )
    .map_err(|e| format!("could not run curl: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "GitHub request failed (curl exit {})",
            output.status
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("unexpected GitHub response: {e}"))?;
    value
        .get("tag_name")
        .and_then(|t| t.as_str())
        .map(|t| t.trim_start_matches('v').to_string())
        .ok_or_else(|| "the GitHub response had no release tag".to_string())
}

/// GET a small file over HTTPS with the bundled curl (used for
/// popup thumbnails). Refuses responses over ~3 MB. Blocking;
/// call on a worker thread. Only http(s) URLs are fetched.
pub fn fetch_url_bytes(url: &str) -> Option<Vec<u8>> {
    fetch_url_bytes_with_cancel(url, &process::CancellationToken::default())
}

pub fn fetch_url_bytes_with_cancel(
    url: &str,
    token: &process::CancellationToken,
) -> Option<Vec<u8>> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }
    let curl = bundled_tool("curl");
    let output = capture(
        &process::ProcessSpec::new(&curl).args([
            "-L",
            "--fail",
            "--max-filesize",
            "3000000",
            "--max-time",
            "10",
            "-H",
            "User-Agent: ytTrayDL",
            "-o",
            "-",
            url,
        ]),
        10,
        3000000,
        token,
    )
    .ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(output.stdout)
}

/// Probe the newest entries of a channel/playlist page with
/// yt-dlp's flat playlist mode (blocking; worker thread). The
/// channel watcher diffs these against its seen list. Any
/// failure yields an empty vec — the watcher just tries again
/// next interval.
pub fn fetch_playlist_entries(
    bin: &std::path::Path,
    settings: &settings::Settings,
    url: &str,
    limit: u32,
) -> Vec<ytdlp::PlaylistEntry> {
    fetch_playlist_entries_with_cancel(
        bin,
        settings,
        url,
        limit,
        &process::CancellationToken::default(),
    )
}

pub fn fetch_playlist_entries_with_cancel(
    bin: &std::path::Path,
    settings: &settings::Settings,
    url: &str,
    limit: u32,
    token: &process::CancellationToken,
) -> Vec<ytdlp::PlaylistEntry> {
    fetch_playlist_entries_with_limits(
        bin,
        settings,
        url,
        limit,
        token,
        process::ProcessLimits::default(),
    )
}

/// Probe with caller-supplied resource limits (including shorter test deadlines).
pub fn fetch_playlist_entries_with_limits(
    bin: &std::path::Path,
    settings: &settings::Settings,
    url: &str,
    limit: u32,
    token: &process::CancellationToken,
    limits: process::ProcessLimits,
) -> Vec<ytdlp::PlaylistEntry> {
    let args = ytdlp::build_playlist_probe_args(settings, url, limit);
    let Ok(output) =
        process::run_capture(&process::ProcessSpec::new(bin).args(&args), limits, token)
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    ytdlp::parse_playlist_entries(&text)
}

/// Download the GyanD essentials ffmpeg build into the managed
/// bin folder: bundled curl.exe fetches the zip, bundled tar.exe
/// (bsdtar) unpacks it, and ffmpeg.exe + ffprobe.exe are moved
/// into place. Blocking; call on a worker thread. Windows only.
/// Returns the installed ffmpeg.exe path.
pub fn download_managed_ffmpeg() -> Result<std::path::PathBuf, String> {
    download_managed_ffmpeg_with_cancel(&process::CancellationToken::default())
}

pub fn download_managed_ffmpeg_with_cancel(
    token: &process::CancellationToken,
) -> Result<std::path::PathBuf, String> {
    if !cfg!(windows) {
        return Err("the in-app ffmpeg install is only supported on Windows".into());
    }
    let dir = settings::Settings::managed_bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip_path = dir.join("ffmpeg.download.zip");
    let extract_dir = dir.join("ffmpeg-extract");
    let _cleanup = TemporaryFiles(vec![zip_path.clone(), extract_dir.clone()]);
    let _ = std::fs::remove_file(&zip_path);
    let _ = std::fs::remove_dir_all(&extract_dir);

    let curl = bundled_tool("curl");
    let status = capture(
        &process::ProcessSpec::new(&curl).args([
            "-L",
            "--fail",
            "-o",
            &zip_path.to_string_lossy(),
            ytdlp::FFMPEG_WINDOWS_URL,
        ]),
        600,
        1048576,
        token,
    )
    .map(|output| output.status)
    .map_err(|e| format!("could not run curl: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&zip_path);
        return Err(format!("ffmpeg download failed (curl exit {status})"));
    }

    std::fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;
    let tar = bundled_tool("tar");
    let status = capture(
        &process::ProcessSpec::new(&tar).args([
            "-xf",
            &zip_path.to_string_lossy(),
            "-C",
            &extract_dir.to_string_lossy(),
        ]),
        120,
        1048576,
        token,
    )
    .map(|output| output.status)
    .map_err(|e| format!("could not run tar: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&zip_path);
        let _ = std::fs::remove_dir_all(&extract_dir);
        return Err(format!(
            "could not unpack the ffmpeg archive (tar exit {status})"
        ));
    }

    let found = ytdlp::find_ffmpeg_exes(&extract_dir);
    let (ffmpeg_src, probe_src) = match found {
        Some(pair) => pair,
        None => {
            let _ = std::fs::remove_file(&zip_path);
            let _ = std::fs::remove_dir_all(&extract_dir);
            return Err("the downloaded archive did not contain ffmpeg.exe where expected".into());
        }
    };
    let target = dir.join("ffmpeg.exe");
    std::fs::rename(&ffmpeg_src, &target).map_err(|e| format!("installing ffmpeg.exe: {e}"))?;
    if let Some(probe) = probe_src {
        let _ = std::fs::rename(probe, dir.join("ffprobe.exe"));
    }
    let _ = std::fs::remove_file(&zip_path);
    let _ = std::fs::remove_dir_all(&extract_dir);
    Ok(target)
}

#[cfg(test)]
mod operation_tests {
    use super::*;
    #[test]
    fn failed_install_preserves_existing_and_removes_partial() {
        let dir = std::env::temp_dir().join(format!("yttraydl-install-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("installed");
        let temporary = dir.join("partial");
        std::fs::write(&target, b"existing").unwrap();
        let result = install_downloaded(&target, &temporary, |path| {
            std::fs::write(path, b"incomplete").unwrap();
            Err("injected process failure".into())
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"existing");
        assert!(!temporary.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
