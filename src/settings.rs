//! Persistent user settings.
//!
//! Settings live as JSON in the per-user config directory
//! (`%APPDATA%\ytTrayDL\settings.json` on Windows). Every field has a
//! serde default so a settings file written by an older version keeps
//! loading after an upgrade, and a corrupt file never prevents startup:
//! it is renamed aside and defaults are used.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What ytTrayDL does when it sees a supported URL on the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DefaultAction {
    /// Show the small download popup and let the user choose.
    #[default]
    Prompt,
    /// Download immediately with the default preset, no popup.
    AutoDownload,
}

/// What happens when the user closes the main (panel) window.
/// Chosen in the close dialog (with a "remember" option) or in
/// Settings → General. Auxiliary windows (Settings, Downloads…)
/// always just close; this governs the main window only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloseAction {
    /// Ask every time (the dialog offers to remember a choice).
    #[default]
    Ask,
    /// Quit ytTrayDL completely.
    Exit,
    /// Close the window; keep running with the tray icon visible.
    Tray,
    /// Minimize the window to the taskbar like a normal app.
    Taskbar,
    /// Close the window AND hide the tray icon; keep running in
    /// the background. Ctrl+Alt+G, relaunching, or `--show`
    /// brings everything back.
    Hide,
}

impl CloseAction {
    pub fn label(&self) -> &'static str {
        match self {
            CloseAction::Ask => "Ask every time",
            CloseAction::Exit => "Exit ytTrayDL",
            CloseAction::Tray => "Minimize to tray",
            CloseAction::Taskbar => "Minimize to taskbar",
            CloseAction::Hide => "Hide entirely (background)",
        }
    }

    pub const ALL: [CloseAction; 5] = [
        CloseAction::Ask,
        CloseAction::Exit,
        CloseAction::Tray,
        CloseAction::Taskbar,
        CloseAction::Hide,
    ];
}

/// Which URLs count as "supported".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DetectionScope {
    /// Only hosts on the built-in known-sites list (plus the user's
    /// own allow list). Quietest option — ordinary web links are ignored.
    #[default]
    KnownSitesOnly,
    /// Any http(s) URL that is not ignored. yt-dlp supports 1,000+
    /// sites including many long-tail ones, so this catches more,
    /// at the cost of popups for links yt-dlp may not handle.
    AllUrls,
    /// Only hosts on the user's allow list.
    AllowListOnly,
}

/// A named quality/format choice. The preset list is deliberately
/// short; power users can pick `Custom` and supply a raw yt-dlp
/// format string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    #[default]
    Best,
    P2160,
    P1440,
    P1080,
    P720,
    P480,
    P360,
    /// Best audio, extracted and converted to MP3 (needs ffmpeg).
    AudioMp3,
    /// Best audio in its original container, no conversion.
    AudioOriginal,
    /// Use `Settings::custom_format` verbatim as the `-f` argument.
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 10] = [
        Preset::Best,
        Preset::P2160,
        Preset::P1440,
        Preset::P1080,
        Preset::P720,
        Preset::P480,
        Preset::P360,
        Preset::AudioMp3,
        Preset::AudioOriginal,
        Preset::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Best => "Best quality",
            Preset::P2160 => "Up to 2160p (4K)",
            Preset::P1440 => "Up to 1440p",
            Preset::P1080 => "Up to 1080p",
            Preset::P720 => "Up to 720p",
            Preset::P480 => "Up to 480p",
            Preset::P360 => "Up to 360p",
            Preset::AudioMp3 => "Audio only — MP3",
            Preset::AudioOriginal => "Audio only — original format",
            Preset::Custom => "Custom format string…",
        }
    }

    pub fn is_audio(self) -> bool {
        matches!(self, Preset::AudioMp3 | Preset::AudioOriginal)
    }

    /// Max video height for capped presets.
    pub fn max_height(self) -> Option<u32> {
        match self {
            Preset::P2160 => Some(2160),
            Preset::P1440 => Some(1440),
            Preset::P1080 => Some(1080),
            Preset::P720 => Some(720),
            Preset::P480 => Some(480),
            Preset::P360 => Some(360),
            _ => None,
        }
    }
}

/// Preferred output container for merged video downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    #[default]
    Mp4,
    Mkv,
    /// Keep whatever yt-dlp produces; no merge-output-format is passed.
    Original,
}

impl Container {
    pub fn label(self) -> &'static str {
        match self {
            Container::Mp4 => "MP4",
            Container::Mkv => "MKV",
            Container::Original => "Original (no remux)",
        }
    }
    pub fn merge_format(self) -> Option<&'static str> {
        match self {
            Container::Mp4 => Some("mp4"),
            Container::Mkv => Some("mkv"),
            Container::Original => None,
        }
    }
}

/// Where the download popup appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PopupPosition {
    #[default]
    BottomRight,
    NearCursor,
    TopRight,
    TopLeft,
    BottomLeft,
    Center,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,

    // --- Monitoring / tray behaviour ---
    pub monitoring_enabled: bool,
    /// When false the tray icon is hidden. The app keeps running;
    /// relaunching the exe, pressing the show hotkey, or running with
    /// `--show` brings the icon and settings back. The UI always
    /// explains this before hiding.
    pub tray_icon_visible: bool,
    pub start_with_windows: bool,
    pub default_action: DefaultAction,
    pub detection_scope: DetectionScope,
    /// What closing the main window does. `Ask` (the default) shows
    /// the choice dialog with a "remember" option; the choice can
    /// be changed later in Settings → General.
    pub close_action: CloseAction,

    // --- Domain rules (host names, lowercase, no scheme) ---
    /// Matching URLs are never prompted or downloaded.
    pub ignored_domains: Vec<String>,
    /// Matching URLs download automatically even in Prompt mode.
    pub auto_domains: Vec<String>,
    /// Extra hosts treated as supported under KnownSitesOnly, and the
    /// only hosts treated as supported under AllowListOnly.
    pub allowed_domains: Vec<String>,
    /// Specific URLs the user chose to never be asked about again.
    pub ignored_urls: Vec<String>,

    // --- Deduplication ---
    /// Re-copying the same URL within this window does not re-trigger.
    pub dedupe_window_secs: u64,
    /// Skip URLs already recorded in the yt-dlp download archive.
    pub skip_already_downloaded: bool,

    // --- Download defaults ---
    pub download_folder: PathBuf,
    /// yt-dlp output template (file name part only).
    pub filename_template: String,
    pub default_preset: Preset,
    pub container: Container,
    pub custom_format: String,
    pub embed_thumbnail: bool,
    pub embed_metadata: bool,
    pub write_subtitles: bool,
    pub subtitle_languages: String,
    pub allow_playlists: bool,
    pub max_concurrent_downloads: usize,
    /// Bandwidth cap for downloads in KiB/s (passed to yt-dlp as
    /// `--limit-rate`). 0 = no limit.
    pub rate_limit_kbps: u32,
    /// Automatically retry a failed download this many times
    /// (with its original preset) before giving up. 0 = off.
    pub auto_retry_attempts: u32,
    /// Extra yt-dlp arguments, split like a command line. Power-user
    /// escape hatch, shown verbatim in Settings.
    pub extra_args: String,
    /// e.g. "chrome", "firefox", "edge" — passed as
    /// `--cookies-from-browser`. Empty = no cookies.
    pub cookies_from_browser: String,

    // --- yt-dlp location ---
    /// Explicit path to yt-dlp.exe; empty = auto-locate (next to
    /// ytTrayDL, then ytTrayDL's managed bin folder, then PATH).
    pub ytdlp_path: PathBuf,
    pub ffmpeg_path: PathBuf,

    // --- Notifications / popup ---
    pub notify_on_complete: bool,
    /// Open the download folder in Explorer when a download
    /// finishes successfully.
    pub open_folder_on_finish: bool,
    pub notify_on_error: bool,
    pub notify_on_auto_start: bool,
    pub popup_position: PopupPosition,
    /// Popup auto-dismisses after this many seconds (0 = never).
    /// Downloads never auto-start on timeout; dismissal = "not now".
    pub popup_timeout_secs: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            monitoring_enabled: true,
            tray_icon_visible: true,
            start_with_windows: false,
            default_action: DefaultAction::Prompt,
            detection_scope: DetectionScope::KnownSitesOnly,
            close_action: CloseAction::Ask,
            ignored_domains: Vec::new(),
            auto_domains: Vec::new(),
            allowed_domains: Vec::new(),
            ignored_urls: Vec::new(),
            dedupe_window_secs: 300,
            skip_already_downloaded: true,
            download_folder: default_download_folder(),
            filename_template: "%(title)s [%(id)s].%(ext)s".to_string(),
            default_preset: Preset::default(),
            container: Container::default(),
            custom_format: "bv*+ba/b".to_string(),
            embed_thumbnail: true,
            embed_metadata: true,
            write_subtitles: false,
            subtitle_languages: "en.*".to_string(),
            allow_playlists: false,
            max_concurrent_downloads: 2,
            rate_limit_kbps: 0,
            auto_retry_attempts: 0,
            extra_args: String::new(),
            cookies_from_browser: String::new(),
            ytdlp_path: PathBuf::new(),
            ffmpeg_path: PathBuf::new(),
            notify_on_complete: true,
            open_folder_on_finish: false,
            notify_on_error: true,
            notify_on_auto_start: false,
            popup_position: PopupPosition::default(),
            popup_timeout_secs: 30,
        }
    }
}

/// The user's Videos folder when we can find one, else Downloads,
/// else the home directory. Resolved without any platform crate so
/// the logic stays testable: callers may override with the real
/// Windows known-folder path at startup.
pub fn default_download_folder() -> PathBuf {
    if let Some(home) = home_dir() {
        let videos = home.join("Videos");
        if videos.is_dir() {
            return videos.join("ytTrayDL");
        }
        let downloads = home.join("Downloads");
        if downloads.is_dir() {
            return downloads.join("ytTrayDL");
        }
        return home.join("ytTrayDL");
    }
    PathBuf::from("ytTrayDL Downloads")
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
}

/// `%APPDATA%\ytTrayDL` on Windows, `~/.config/yttraydl` elsewhere.
pub fn config_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("ytTrayDL");
    }
    if let Some(home) = home_dir() {
        return home.join(".config").join("yttraydl");
    }
    PathBuf::from(".yttraydl")
}

impl Settings {
    pub fn settings_path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn archive_path() -> PathBuf {
        config_dir().join("download-archive.txt")
    }

    pub fn history_path() -> PathBuf {
        config_dir().join("history.json")
    }

    pub fn managed_bin_dir() -> PathBuf {
        config_dir().join("bin")
    }

    /// Load settings; on any error return defaults. A corrupt file is
    /// preserved as `settings.json.bad` for diagnosis.
    pub fn load() -> Self {
        let path = Self::settings_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.normalize();
                s
            }
            Err(_) => {
                let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                Self::default()
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // Write-then-rename so a crash mid-save can't corrupt settings.
        let tmp = dir.join("settings.json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, Self::settings_path())
    }

    fn normalize(&mut self) {
        let norm = |v: &mut Vec<String>| {
            for s in v.iter_mut() {
                *s = normalize_domain(s);
            }
            v.retain(|s| !s.is_empty());
            v.sort();
            v.dedup();
        };
        norm(&mut self.ignored_domains);
        norm(&mut self.auto_domains);
        norm(&mut self.allowed_domains);
        self.ignored_urls.retain(|u| !u.trim().is_empty());
        self.max_concurrent_downloads = self.max_concurrent_downloads.clamp(1, 8);
        if self.filename_template.trim().is_empty() {
            self.filename_template = "%(title)s [%(id)s].%(ext)s".to_string();
        }
    }
}

/// Normalize a user-typed domain rule to a bare lowercase host:
/// strips scheme, path, port, leading `*.` / `.` and whitespace.
pub fn normalize_domain(input: &str) -> String {
    let mut s = input.trim().to_lowercase();
    if let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    {
        s = rest.to_string();
    }
    if let Some(pos) = s.find(['/', '?', '#']) {
        s.truncate(pos);
    }
    if let Some(pos) = s.rfind(':') {
        // Strip a port, but don't mangle an IPv6 literal (not expected
        // in domain rules; those are handled by exact match anyway).
        if !s.contains('[') {
            s.truncate(pos);
        }
    }
    while let Some(rest) = s.strip_prefix("*.").or_else(|| s.strip_prefix('.')) {
        s = rest.to_string();
    }
    s.trim_end_matches('.').to_string()
}

/// Does `host` match a domain rule? A rule matches the host itself or
/// any subdomain of it (`youtube.com` matches `music.youtube.com`).
pub fn domain_matches(host: &str, rule: &str) -> bool {
    let host = host.to_lowercase();
    let rule = normalize_domain(rule);
    if rule.is_empty() {
        return false;
    }
    host == rule || host.ends_with(&format!(".{rule}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert!(s.monitoring_enabled);
        assert!(s.tray_icon_visible);
        assert_eq!(s.default_action, DefaultAction::Prompt);
        assert_eq!(s.default_preset, Preset::Best);
        assert!(s.max_concurrent_downloads >= 1);
    }

    #[test]
    fn json_round_trip() {
        let s = Settings {
            ignored_domains: vec!["example.com".into()],
            default_action: DefaultAction::AutoDownload,
            ..Settings::default()
        };
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.ignored_domains, s.ignored_domains);
        assert_eq!(back.default_action, DefaultAction::AutoDownload);
    }

    #[test]
    fn old_json_without_new_fields_still_loads() {
        let back: Settings = serde_json::from_str(r#"{"monitoring_enabled":false}"#).unwrap();
        assert!(!back.monitoring_enabled);
        assert!(back.tray_icon_visible); // default filled in
    }

    #[test]
    fn normalize_domain_strips_decoration() {
        assert_eq!(
            normalize_domain("  HTTPS://*.YouTube.com/watch?v=x "),
            "youtube.com"
        );
        assert_eq!(normalize_domain("vimeo.com:443"), "vimeo.com");
        assert_eq!(normalize_domain(".tiktok.com."), "tiktok.com");
        assert_eq!(normalize_domain(""), "");
    }

    #[test]
    fn domain_matching_covers_subdomains_only() {
        assert!(domain_matches("music.youtube.com", "youtube.com"));
        assert!(domain_matches("youtube.com", "youtube.com"));
        assert!(!domain_matches("notyoutube.com", "youtube.com"));
        assert!(!domain_matches("youtube.com.evil.example", "youtube.com"));
    }

    #[test]
    fn close_action_defaults_to_ask_and_roundtrips() {
        // Default settings ask — never silently exit or hide.
        assert_eq!(Settings::default().close_action, CloseAction::Ask);
        // Old settings files (no close_action key) still load.
        let legacy: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.close_action, CloseAction::Ask);
        for a in CloseAction::ALL {
            let json = serde_json::to_string(&a).unwrap();
            assert_eq!(serde_json::from_str::<CloseAction>(&json).unwrap(), a);
        }
        assert_eq!(
            serde_json::to_string(&CloseAction::Hide).unwrap(),
            "\"hide\""
        );
        assert_eq!(CloseAction::ALL.len(), 5);
    }
}
