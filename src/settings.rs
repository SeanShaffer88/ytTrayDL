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
    /// Best audio, extracted and converted to M4A (needs ffmpeg).
    AudioM4a,
    /// Best audio, extracted and converted to Opus (needs ffmpeg).
    AudioOpus,
    /// Best audio, extracted and converted to FLAC (needs ffmpeg).
    AudioFlac,
    /// Best audio, extracted and converted to WAV (needs ffmpeg).
    AudioWav,
    /// Best audio in its original container, no conversion.
    AudioOriginal,
    /// Subtitle files only — no audio or video is downloaded
    /// (`--skip-download` + subtitle writing, converted to SRT).
    SubtitlesOnly,
    /// Use `Settings::custom_format` verbatim as the `-f` argument.
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 15] = [
        Preset::Best,
        Preset::P2160,
        Preset::P1440,
        Preset::P1080,
        Preset::P720,
        Preset::P480,
        Preset::P360,
        Preset::AudioMp3,
        Preset::AudioM4a,
        Preset::AudioOpus,
        Preset::AudioFlac,
        Preset::AudioWav,
        Preset::AudioOriginal,
        Preset::SubtitlesOnly,
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
            Preset::AudioM4a => "Audio only — M4A",
            Preset::AudioOpus => "Audio only — Opus",
            Preset::AudioFlac => "Audio only — FLAC",
            Preset::AudioWav => "Audio only — WAV",
            Preset::AudioOriginal => "Audio only — original format",
            Preset::SubtitlesOnly => "Subtitles only (no video/audio)",
            Preset::Custom => "Custom format string…",
        }
    }

    pub fn is_audio(self) -> bool {
        matches!(
            self,
            Preset::AudioMp3
                | Preset::AudioM4a
                | Preset::AudioOpus
                | Preset::AudioFlac
                | Preset::AudioWav
                | Preset::AudioOriginal
        )
    }

    /// The `--audio-format` value for the extract-and-convert
    /// audio presets. `AudioOriginal` converts nothing (`None`).
    pub fn audio_format(self) -> Option<&'static str> {
        match self {
            Preset::AudioMp3 => Some("mp3"),
            Preset::AudioM4a => Some("m4a"),
            Preset::AudioOpus => Some("opus"),
            Preset::AudioFlac => Some("flac"),
            Preset::AudioWav => Some("wav"),
            _ => None,
        }
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

/// A per-domain quality pin (Settings → Domains & rules).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainPreset {
    /// Bare lowercase host (see `normalize_domain`); matches
    /// subdomains like every other domain rule.
    pub domain: String,
    pub preset: Preset,
}

/// A channel/playlist the watcher polls for new uploads
/// (Settings → Domains & rules → Channel watcher). New entries
/// download automatically with the subscription's preset; the
/// seen-entry bookkeeping lives in `watch.json`, not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    /// Channel or playlist page URL, as pasted by the user.
    pub url: String,
    #[serde(default)]
    pub preset: Preset,
}

/// What ytTrayDL does when the download queue drains to empty
/// after having been active (Settings → General).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum QueueEmptyAction {
    #[default]
    Nothing,
    /// Play the notification sound.
    Sound,
    /// Put the PC to sleep.
    Sleep,
    /// Quit ytTrayDL.
    ExitApp,
}

impl QueueEmptyAction {
    pub fn label(&self) -> &'static str {
        match self {
            QueueEmptyAction::Nothing => "Nothing",
            QueueEmptyAction::Sound => "Play a sound",
            QueueEmptyAction::Sleep => "Sleep the PC",
            QueueEmptyAction::ExitApp => "Exit ytTrayDL",
        }
    }

    pub const ALL: [QueueEmptyAction; 4] = [
        QueueEmptyAction::Nothing,
        QueueEmptyAction::Sound,
        QueueEmptyAction::Sleep,
        QueueEmptyAction::ExitApp,
    ];
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
    /// Animate the Settings drawer sliding open/closed. When
    /// false, the drawer (and the window resize) is instant.
    pub animate_settings_drawer: bool,

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
    /// Per-domain quality pins: links from these domains use the
    /// pinned preset instead of the global default (the popup
    /// still lets the user change it per download).
    pub domain_presets: Vec<DomainPreset>,

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
    /// Remove SponsorBlock-marked segments (sponsor, self-promo,
    /// interaction reminders…) from downloads via yt-dlp's
    /// `--sponsorblock-remove all`. Needs ffmpeg.
    pub sponsorblock_remove: bool,
    /// Split chaptered downloads into one file per chapter
    /// (`--split-chapters`). Needs ffmpeg.
    pub split_chapters: bool,
    /// What to do when the queue drains to empty after activity.
    pub queue_empty_action: QueueEmptyAction,
    /// Pause starting new downloads while the download drive has
    /// less free space than this (MiB). 0 = guard off.
    pub low_disk_threshold_mb: u64,
    /// A command line run after each successful download, with
    /// `{file}`, `{dir}`, `{url}` and `{title}` substituted.
    /// Empty = no command. Runs through the system shell.
    pub post_download_command: String,
    /// Channel watcher poll interval in minutes. 0 = watcher off.
    pub watch_interval_mins: u32,
    /// Channels/playlists the watcher polls for new uploads.
    pub subscriptions: Vec<Subscription>,
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
    /// Play the Windows notification sound when a download
    /// finishes successfully.
    pub sound_on_finish: bool,
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
            animate_settings_drawer: true,
            ignored_domains: Vec::new(),
            auto_domains: Vec::new(),
            allowed_domains: Vec::new(),
            ignored_urls: Vec::new(),
            domain_presets: Vec::new(),
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
            sponsorblock_remove: false,
            split_chapters: false,
            queue_empty_action: QueueEmptyAction::Nothing,
            low_disk_threshold_mb: 0,
            post_download_command: String::new(),
            watch_interval_mins: 60,
            subscriptions: Vec::new(),
            extra_args: String::new(),
            cookies_from_browser: String::new(),
            ytdlp_path: PathBuf::new(),
            ffmpeg_path: PathBuf::new(),
            notify_on_complete: true,
            sound_on_finish: false,
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

/// Portable mode: when a file named `portable.txt` sits next to
/// the executable, ALL of ytTrayDL's data (settings, history,
/// archive, managed binaries) lives in a `ytTrayDL-data` folder
/// beside the exe instead of the per-user config dir, so the
/// portable zip is truly self-contained. Detected once.
pub fn portable_data_dir() -> Option<PathBuf> {
    static CACHE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let dir = exe.parent()?;
            dir.join("portable.txt")
                .is_file()
                .then(|| dir.join("ytTrayDL-data"))
        })
        .clone()
}

/// `%APPDATA%\ytTrayDL` on Windows, `~/.config/yttraydl`
/// elsewhere — or the portable data dir in portable mode. Every
/// stored file resolves through here, so portable mode is this
/// one decision.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = portable_data_dir() {
        return dir;
    }
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

    /// Parse settings from JSON text (the Import feature), with
    /// the same normalization `load` applies. The caller replaces
    /// the live settings with the result.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut s: Settings =
            serde_json::from_str(text).map_err(|e| format!("not a valid settings file: {e}"))?;
        s.normalize();
        Ok(s)
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
        for dp in &mut self.domain_presets {
            dp.domain = normalize_domain(&dp.domain);
        }
        self.domain_presets.retain(|dp| !dp.domain.is_empty());
        self.domain_presets.sort_by(|a, b| a.domain.cmp(&b.domain));
        self.domain_presets.dedup_by(|a, b| a.domain == b.domain);
        self.ignored_urls.retain(|u| !u.trim().is_empty());
        for sub in &mut self.subscriptions {
            sub.url = sub.url.trim().to_string();
        }
        self.subscriptions.retain(|s| !s.url.is_empty());
        self.subscriptions.dedup_by(|a, b| a.url == b.url);
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
