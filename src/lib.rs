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
pub mod platform;
pub mod rules;
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
    let args = ytdlp::build_info_args(settings, url);
    let output = std::process::Command::new(bin).args(&args).output().ok()?;
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

/// Download the official standalone yt-dlp.exe into the managed bin
/// folder using Windows' bundled curl.exe. Blocking; call on a
/// worker thread. Only supported on Windows builds.
pub fn download_managed_ytdlp() -> Result<std::path::PathBuf, String> {
    let dir = settings::Settings::managed_bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let target = dir.join(if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    });
    let tmp = dir.join("yt-dlp.download");
    let curl = if cfg!(windows) {
        // curl.exe ships in System32 on Windows 10 1803+ / Windows 11.
        let system32 = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let p = std::path::PathBuf::from(system32)
            .join("System32")
            .join("curl.exe");
        if p.is_file() {
            p
        } else {
            std::path::PathBuf::from("curl.exe")
        }
    } else {
        std::path::PathBuf::from("curl")
    };
    let status = std::process::Command::new(&curl)
        .args([
            "-L",
            "--fail",
            "-o",
            &tmp.to_string_lossy(),
            ytdlp::YTDLP_WINDOWS_URL,
        ])
        .status()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("download failed (curl exit {status})"));
    }
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    Ok(target)
}
