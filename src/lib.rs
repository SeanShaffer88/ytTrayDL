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
    let curl = bundled_tool("curl");
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

/// The latest official yt-dlp release tag (e.g. "2025.09.26")
/// from the GitHub API, via the bundled curl. Blocking; call on
/// a worker thread. Used by the "Check for yt-dlp updates"
/// button — ytTrayDL itself never phones home unasked.
pub fn fetch_latest_ytdlp_tag() -> Result<String, String> {
    let curl = bundled_tool("curl");
    let output = std::process::Command::new(&curl)
        .args([
            "-L",
            "--fail",
            "-H",
            "User-Agent: ytTrayDL",
            "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest",
        ])
        .output()
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

/// Download the GyanD essentials ffmpeg build into the managed
/// bin folder: bundled curl.exe fetches the zip, bundled tar.exe
/// (bsdtar) unpacks it, and ffmpeg.exe + ffprobe.exe are moved
/// into place. Blocking; call on a worker thread. Windows only.
/// Returns the installed ffmpeg.exe path.
pub fn download_managed_ffmpeg() -> Result<std::path::PathBuf, String> {
    if !cfg!(windows) {
        return Err("the in-app ffmpeg install is only supported on Windows".into());
    }
    let dir = settings::Settings::managed_bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip_path = dir.join("ffmpeg.download.zip");
    let extract_dir = dir.join("ffmpeg-extract");
    let _ = std::fs::remove_file(&zip_path);
    let _ = std::fs::remove_dir_all(&extract_dir);

    let curl = bundled_tool("curl");
    let status = std::process::Command::new(&curl)
        .args([
            "-L",
            "--fail",
            "-o",
            &zip_path.to_string_lossy(),
            ytdlp::FFMPEG_WINDOWS_URL,
        ])
        .status()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&zip_path);
        return Err(format!("ffmpeg download failed (curl exit {status})"));
    }

    std::fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;
    let tar = bundled_tool("tar");
    let status = std::process::Command::new(&tar)
        .args([
            "-xf",
            &zip_path.to_string_lossy(),
            "-C",
            &extract_dir.to_string_lossy(),
        ])
        .status()
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
