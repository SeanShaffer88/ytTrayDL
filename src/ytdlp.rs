//! Everything about talking to yt-dlp: locating the binary,
//! building its command line from settings, parsing its progress
//! output, and parsing `--dump-single-json` info.
//!
//! ytTrayDL never reimplements downloading — yt-dlp is the engine.
//! The official release is a single self-contained `yt-dlp.exe`
//! (github.com/yt-dlp/yt-dlp releases); ffmpeg is only needed for
//! merging separate video+audio streams and for MP3 conversion,
//! both of which yt-dlp delegates to it.

use crate::settings::{Preset, Settings};
use serde::Deserialize;
use std::path::PathBuf;

/// Where a yt-dlp binary was found, in search order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YtDlpSource {
    /// Path the user set explicitly in Settings.
    UserConfigured,
    /// `yt-dlp.exe` sitting next to yttraydl.exe.
    BesideExe,
    /// ytTrayDL's managed copy in `%APPDATA%\ytTrayDL\bin` —
    /// downloaded (with the user's approval) from the official
    /// GitHub release, and the target of "Update yt-dlp".
    Managed,
    /// Found on PATH.
    Path,
}

#[derive(Debug, Clone)]
pub struct YtDlpBinary {
    pub path: PathBuf,
    pub source: YtDlpSource,
}

/// Locate yt-dlp. Pure with respect to its inputs (the caller passes
/// the exe directory) apart from PATH lookup and existence checks.
pub fn locate_ytdlp(settings: &Settings, exe_dir: Option<&std::path::Path>) -> Option<YtDlpBinary> {
    let file = if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };

    if !settings.ytdlp_path.as_os_str().is_empty() && settings.ytdlp_path.is_file() {
        return Some(YtDlpBinary {
            path: settings.ytdlp_path.clone(),
            source: YtDlpSource::UserConfigured,
        });
    }
    if let Some(dir) = exe_dir {
        let candidate = dir.join(file);
        if candidate.is_file() {
            return Some(YtDlpBinary {
                path: candidate,
                source: YtDlpSource::BesideExe,
            });
        }
    }
    let managed = Settings::managed_bin_dir().join(file);
    if managed.is_file() {
        return Some(YtDlpBinary {
            path: managed,
            source: YtDlpSource::Managed,
        });
    }
    if let Some(path) = find_on_path(file) {
        return Some(YtDlpBinary {
            path,
            source: YtDlpSource::Path,
        });
    }
    None
}

pub fn find_on_path(file: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(file);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Is ffmpeg available (explicit path, beside the exe, managed bin,
/// or PATH)? Drives an honest warning: without it, separate
/// video+audio streams can't merge and MP3 extraction can't convert.
pub fn ffmpeg_available(settings: &Settings, exe_dir: Option<&std::path::Path>) -> bool {
    let file = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if !settings.ffmpeg_path.as_os_str().is_empty() {
        return settings.ffmpeg_path.is_file();
    }
    if let Some(dir) = exe_dir {
        if dir.join(file).is_file() {
            return true;
        }
    }
    if Settings::managed_bin_dir().join(file).is_file() {
        return true;
    }
    find_on_path(file).is_some()
}

/// The official download URL for the standalone Windows build.
pub const YTDLP_WINDOWS_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";

/// Where the in-app ffmpeg install downloads from: the GyanD
/// "essentials" build — the Windows build ffmpeg.org itself
/// links to. The URL redirects to the current versioned package
/// (a zip with `bin/ffmpeg.exe` + `bin/ffprobe.exe` inside a
/// top-level folder). ~110 MB; static, no DLLs needed.
pub const FFMPEG_WINDOWS_URL: &str =
    "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";

/// Find ffmpeg.exe (+ ffprobe.exe when present) inside an
/// extracted ffmpeg distribution tree. Handles the packaged
/// layout (`<root>/bin/ffmpeg.exe`), the package nested one
/// level down (`<root>/<package>/bin/ffmpeg.exe`), and a bare
/// `ffmpeg.exe` sitting directly in the tree. Pure filesystem
/// scan, depth-limited — unit-tested.
pub fn find_ffmpeg_exes(
    root: &std::path::Path,
) -> Option<(std::path::PathBuf, Option<std::path::PathBuf>)> {
    let exe_name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let probe_name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    // Candidates in preference order: root/bin, root/*/bin, root.
    let mut bin_dirs: Vec<std::path::PathBuf> = vec![root.join("bin"), root.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                bin_dirs.insert(1, p.join("bin"));
            }
        }
    }
    for dir in bin_dirs {
        let ffmpeg = dir.join(exe_name);
        if ffmpeg.is_file() {
            let probe = dir.join(probe_name);
            return Some((ffmpeg, probe.is_file().then_some(probe)));
        }
    }
    None
}

/// Build the argument vector for a download (excluding argv[0]).
/// Order matters to yt-dlp only in that later options can override
/// earlier ones; user `extra_args` go last so they win.
/// `use_archive` is normally `settings.skip_already_downloaded`;
/// a "download anyway" request from the popup passes false.
pub fn build_download_args(
    settings: &Settings,
    preset: Preset,
    url: &str,
    use_archive: bool,
) -> Vec<String> {
    // Machine-parseable, line-at-a-time progress; resume partial
    // downloads instead of restarting them; don't let a page's own
    // timestamps confuse file sorting.
    let mut args: Vec<String> = vec![
        "--newline".into(),
        "--no-colors".into(),
        "--progress".into(),
        "--continue".into(),
        "--no-mtime".into(),
    ];

    // Format selection.
    match preset {
        Preset::Best => {
            args.push("-f".into());
            args.push("bv*+ba/b".into());
        }
        Preset::AudioMp3 => {
            args.push("-x".into());
            args.push("--audio-format".into());
            args.push("mp3".into());
            args.push("--audio-quality".into());
            args.push("0".into());
        }
        Preset::AudioOriginal => {
            args.push("-f".into());
            args.push("ba/b".into());
            args.push("-x".into());
        }
        Preset::Custom => {
            args.push("-f".into());
            args.push(settings.custom_format.clone());
        }
        capped => {
            let h = capped.max_height().unwrap_or(4320);
            args.push("-f".into());
            args.push(format!("bv*[height<={h}]+ba/b[height<={h}]/b"));
        }
    }

    if !preset.is_audio() {
        if let Some(fmt) = settings.container.merge_format() {
            args.push("--merge-output-format".into());
            args.push(fmt.into());
        }
    }

    // Output location + name.
    let template = settings
        .download_folder
        .join(&settings.filename_template)
        .to_string_lossy()
        .to_string();
    args.push("-o".into());
    args.push(template);

    // Duplicate protection across sessions.
    if use_archive {
        args.push("--download-archive".into());
        args.push(Settings::archive_path().to_string_lossy().to_string());
    }

    if settings.allow_playlists {
        args.push("--yes-playlist".into());
    } else {
        args.push("--no-playlist".into());
    }

    if settings.embed_metadata {
        args.push("--embed-metadata".into());
    }
    if settings.embed_thumbnail {
        // Thumbnail embedding works for video containers and for
        // extracted audio (cover art); yt-dlp warns harmlessly when
        // a container can't hold it.
        args.push("--embed-thumbnail".into());
    }
    if settings.write_subtitles {
        args.push("--write-subs".into());
        args.push("--write-auto-subs".into());
        args.push("--sub-langs".into());
        args.push(settings.subtitle_languages.clone());
        if !preset.is_audio() {
            args.push("--embed-subs".into());
        }
    }

    // Effective ffmpeg location: the user's explicit path wins;
    // otherwise the managed copy (Settings' bin folder) is passed
    // explicitly — a yt-dlp installed elsewhere would never look
    // in ytTrayDL's folder on its own. When neither exists we pass
    // nothing and yt-dlp falls back to its own search (beside
    // itself, then PATH).
    if !settings.ffmpeg_path.as_os_str().is_empty() {
        args.push("--ffmpeg-location".into());
        args.push(settings.ffmpeg_path.to_string_lossy().to_string());
    } else {
        let managed = Settings::managed_bin_dir().join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
        if managed.is_file() {
            args.push("--ffmpeg-location".into());
            args.push(managed.to_string_lossy().to_string());
        }
    }

    let cookies = settings.cookies_from_browser.trim();
    if !cookies.is_empty() {
        args.push("--cookies-from-browser".into());
        args.push(cookies.to_string());
    }

    args.extend(split_extra_args(&settings.extra_args));

    args.push("--".into());
    args.push(url.to_string());
    args
}

/// Arguments for the info probe that fills in the popup
/// (title/uploader/duration) without downloading anything.
pub fn build_info_args(settings: &Settings, url: &str) -> Vec<String> {
    let mut args = vec![
        "--dump-single-json".to_string(),
        "--no-warnings".to_string(),
        "--no-colors".to_string(),
    ];
    if settings.allow_playlists {
        args.push("--flat-playlist".into());
    } else {
        args.push("--no-playlist".into());
    }
    let cookies = settings.cookies_from_browser.trim();
    if !cookies.is_empty() {
        args.push("--cookies-from-browser".into());
        args.push(cookies.to_string());
    }
    args.push("--".into());
    args.push(url.to_string());
    args
}

/// Split a user-supplied extra-args string into arguments.
/// Whitespace separates; single and double quotes group. Backslash
/// follows Windows conventions — it is a literal character (paths!)
/// except directly before a quote, where it escapes that quote.
pub fn split_extra_args(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut has_cur = false;
    let mut quote: Option<char> = None;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else if c == '\\' && chars.peek() == Some(&'"') && q == '"' {
                    chars.next();
                    cur.push('"');
                } else {
                    cur.push(c);
                }
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    has_cur = true;
                }
                '\\' if chars.peek() == Some(&'"') || chars.peek() == Some(&'\'') => {
                    let n = chars.next().unwrap();
                    cur.push(n);
                    has_cur = true;
                }
                c if c.is_whitespace() => {
                    if has_cur || !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                        has_cur = false;
                    }
                }
                _ => {
                    cur.push(c);
                    has_cur = true;
                }
            },
        }
    }
    if has_cur || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// One parsed progress/state event from a yt-dlp output line.
#[derive(Debug, Clone, PartialEq)]
pub enum ProgressEvent {
    /// `[download] Destination: <path>`
    Destination(PathBuf),
    /// Percent complete 0.0–100.0 with optional total size text,
    /// speed text and ETA text as yt-dlp printed them.
    Progress {
        percent: f32,
        total: Option<String>,
        speed: Option<String>,
        eta: Option<String>,
    },
    /// The file was already fully present.
    AlreadyDownloaded,
    /// `[Merger] Merging formats into "<path>"` or an
    /// `[ExtractAudio]` destination — post-processing started.
    PostProcessing(String),
    /// Final media file path from `[download] <path> has already…`
    /// style lines is covered by AlreadyDownloaded; the manager
    /// learns the real final path from the process exit + the
    /// last Destination / Merger line.
    Other,
}

/// Parse one line of yt-dlp stdout/stderr. yt-dlp writes progress
/// lines to stderr when not a TTY in some versions, so the caller
/// feeds both streams through here.
pub fn parse_progress_line(line: &str) -> ProgressEvent {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("[download] Destination: ") {
        return ProgressEvent::Destination(PathBuf::from(rest.trim()));
    }
    if line.contains("has already been downloaded") {
        return ProgressEvent::AlreadyDownloaded;
    }
    if let Some(rest) = line.strip_prefix("[Merger] Merging formats into ") {
        return ProgressEvent::PostProcessing(rest.trim().trim_matches('"').to_string());
    }
    if line.starts_with("[ExtractAudio]") || line.starts_with("[ffmpeg]") {
        return ProgressEvent::PostProcessing(line.to_string());
    }
    if let Some(rest) = line.strip_prefix("[download]") {
        // "[download]  42.3% of  123.45MiB at  5.67MiB/s ETA 00:12"
        // "[download] 100% of  123.45MiB in 00:00:23 at 5.67MiB/s"
        let rest = rest.trim();
        if let Some(pct_end) = rest.find('%') {
            if let Ok(percent) = rest[..pct_end].trim().parse::<f32>() {
                let tail = &rest[pct_end + 1..];
                let total = extract_between(tail, " of ", &[" at ", " in "]);
                let speed = extract_between(tail, " at ", &[" ETA "])
                    .or_else(|| extract_between(tail, " at ", &[]));
                let eta = extract_between(tail, " ETA ", &[]);
                return ProgressEvent::Progress {
                    percent: percent.clamp(0.0, 100.0),
                    total,
                    speed,
                    eta,
                };
            }
        }
    }
    ProgressEvent::Other
}

fn extract_between(text: &str, start: &str, ends: &[&str]) -> Option<String> {
    let begin = text.find(start)? + start.len();
    let mut end = text.len();
    for e in ends {
        if let Some(pos) = text[begin..].find(e) {
            end = end.min(begin + pos);
        }
    }
    let value = text[begin..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// The subset of yt-dlp's info JSON the popup and history use.
#[derive(Debug, Clone, Deserialize)]
pub struct VideoInfo {
    pub id: Option<String>,
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub channel: Option<String>,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub extractor_key: Option<String>,
    pub webpage_url: Option<String>,
    #[serde(rename = "_type")]
    pub kind: Option<String>,
    pub playlist_count: Option<u32>,
}

impl VideoInfo {
    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Untitled".to_string())
    }
    pub fn display_uploader(&self) -> Option<String> {
        self.uploader
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.channel.clone().filter(|s| !s.trim().is_empty()))
    }
    pub fn is_playlist(&self) -> bool {
        self.kind.as_deref() == Some("playlist") || self.playlist_count.unwrap_or(0) > 1
    }
}

pub fn parse_info_json(text: &str) -> Result<VideoInfo, serde_json::Error> {
    serde_json::from_str::<VideoInfo>(text)
}

/// Human duration: 3661 → "1:01:01", 61 → "1:01", 9 → "0:09".
pub fn format_duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return String::new();
    }
    let total = seconds.round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Does the yt-dlp download archive already contain this URL's
/// video? The archive stores `<extractor> <id>` lines, so the check
/// needs the info probe's extractor + id. When either is unknown we
/// answer `false` (don't suppress on a guess).
pub fn archive_contains(archive_text: &str, extractor: &str, id: &str) -> bool {
    if extractor.is_empty() || id.is_empty() {
        return false;
    }
    let needle = format!("{} {}", extractor.to_lowercase(), id);
    archive_text
        .lines()
        .any(|l| l.trim().eq_ignore_ascii_case(&needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::default()
    }

    #[test]
    fn find_ffmpeg_exes_handles_packaged_layouts() {
        let exe = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        let probe = if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        };
        let base = std::env::temp_dir().join(format!("yttraydl-ff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        // Nothing there yet.
        std::fs::create_dir_all(&base).unwrap();
        assert!(find_ffmpeg_exes(&base).is_none());

        // Packaged layout: <root>/<package>/bin/ffmpeg.exe (+probe).
        let bin = base.join("ffmpeg-9.0-essentials_build").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(exe), b"x").unwrap();
        let (found, found_probe) = find_ffmpeg_exes(&base).unwrap();
        assert_eq!(found, bin.join(exe));
        assert!(found_probe.is_none(), "no ffprobe in tree yet");
        std::fs::write(bin.join(probe), b"x").unwrap();
        let (_, found_probe) = find_ffmpeg_exes(&base).unwrap();
        assert_eq!(found_probe, Some(bin.join(probe)));

        // Bare layout: ffmpeg.exe directly in the root also works.
        let bare = base.join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(bare.join(exe), b"x").unwrap();
        assert_eq!(find_ffmpeg_exes(&bare).unwrap().0, bare.join(exe));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn best_args_shape() {
        let args = build_download_args(&settings(), Preset::Best, "https://youtu.be/x", true);
        assert!(args.contains(&"-f".to_string()));
        assert!(args.contains(&"bv*+ba/b".to_string()));
        assert!(args.contains(&"--merge-output-format".to_string()));
        assert!(args.contains(&"--no-playlist".to_string()));
        assert_eq!(args.last().unwrap(), "https://youtu.be/x");
        assert_eq!(args[args.len() - 2], "--");
    }

    #[test]
    fn capped_preset_uses_height_filter() {
        let args = build_download_args(&settings(), Preset::P720, "https://youtu.be/x", true);
        assert!(args
            .iter()
            .any(|a| a == "bv*[height<=720]+ba/b[height<=720]/b"));
    }

    #[test]
    fn audio_mp3_extracts_without_merge_format() {
        let args = build_download_args(&settings(), Preset::AudioMp3, "https://youtu.be/x", true);
        assert!(args.contains(&"-x".to_string()));
        assert!(args.contains(&"mp3".to_string()));
        assert!(!args.contains(&"--merge-output-format".to_string()));
    }

    #[test]
    fn extra_args_come_before_url_and_win() {
        let mut s = settings();
        s.extra_args = "--limit-rate 5M --retries 3".into();
        let args = build_download_args(&s, Preset::Best, "https://youtu.be/x", true);
        let pos = args.iter().position(|a| a == "--limit-rate").unwrap();
        assert!(pos < args.len() - 2);
        assert!(args.contains(&"5M".to_string()));
    }

    #[test]
    fn split_extra_args_handles_quotes() {
        assert_eq!(
            split_extra_args(r#"--remux-video mp4 --write-info-json --paths "D:\My Videos""#),
            vec![
                "--remux-video",
                "mp4",
                "--write-info-json",
                "--paths",
                r"D:\My Videos"
            ]
        );
        assert_eq!(split_extra_args(""), Vec::<String>::new());
        assert_eq!(split_extra_args("  --a  'b c' "), vec!["--a", "b c"]);
    }

    #[test]
    fn progress_parsing() {
        match parse_progress_line("[download]  42.3% of  123.45MiB at  5.67MiB/s ETA 00:12") {
            ProgressEvent::Progress {
                percent,
                total,
                speed,
                eta,
            } => {
                assert!((percent - 42.3).abs() < 0.01);
                assert_eq!(total.as_deref(), Some("123.45MiB"));
                assert_eq!(speed.as_deref(), Some("5.67MiB/s"));
                assert_eq!(eta.as_deref(), Some("00:12"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(matches!(
            parse_progress_line("[download] Destination: C:\\Videos\\a.mp4"),
            ProgressEvent::Destination(_)
        ));
        assert_eq!(
            parse_progress_line("[download] C:\\Videos\\a.mp4 has already been downloaded"),
            ProgressEvent::AlreadyDownloaded
        );
        assert_eq!(parse_progress_line("random noise"), ProgressEvent::Other);
    }

    #[test]
    fn info_parsing() {
        let info = parse_info_json(
            r#"{"id":"abc","title":"Hello","uploader":"Chan","duration":61.0,
                "extractor_key":"Youtube","webpage_url":"https://youtu.be/abc"}"#,
        )
        .unwrap();
        assert_eq!(info.display_title(), "Hello");
        assert_eq!(info.display_uploader().as_deref(), Some("Chan"));
        assert_eq!(format_duration(61.0), "1:01");
        assert_eq!(format_duration(3661.0), "1:01:01");
        assert!(!info.is_playlist());
    }

    #[test]
    fn archive_matching() {
        let text = "youtube abc123\nvimeo 999\n";
        assert!(archive_contains(text, "YouTube", "abc123"));
        assert!(!archive_contains(text, "youtube", "nope"));
        assert!(!archive_contains(text, "", "abc123"));
    }
}
