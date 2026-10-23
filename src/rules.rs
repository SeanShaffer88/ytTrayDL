//! The decision engine: given a detected URL and the user's
//! settings, decide what ytTrayDL should do. Pure logic, no I/O —
//! the archive-membership fact is passed in by the caller.
//!
//! Priority, highest first:
//! 1. URL is on the user's ignored-URL list          → Ignore
//! 2. Host matches an ignored domain                 → Ignore
//! 3. URL was already downloaded (archive hit)       → Ignore
//! 4. Host is outside the detection scope            → Ignore
//! 5. Host matches an auto-download domain           → AutoDownload
//! 6. The global default action                      → Prompt / AutoDownload

use crate::settings::{domain_matches, DefaultAction, DetectionScope, Preset, Settings};
use crate::url_detect::{host_of, is_known_site, normalize_url};

/// The per-domain quality pin for a URL, if one applies
/// (Settings → Domains & rules → Quality by domain). Used as
/// the starting preset for both the popup and auto-downloads;
/// `None` means "use the global default preset".
pub fn preset_for_url(url: &str, settings: &Settings) -> Option<Preset> {
    let host = host_of(url);
    if host.is_empty() {
        return None;
    }
    settings
        .domain_presets
        .iter()
        .find(|dp| domain_matches(&host, &dp.domain))
        .map(|dp| dp.preset)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Prompt,
    AutoDownload,
    Ignore(IgnoreReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IgnoreReason {
    IgnoredUrl,
    IgnoredDomain(String),
    AlreadyDownloaded,
    OutOfScope(String),
}

impl IgnoreReason {
    pub fn describe(&self) -> String {
        match self {
            IgnoreReason::IgnoredUrl => "on your ignore list".into(),
            IgnoreReason::IgnoredDomain(d) => format!("domain {d} is ignored"),
            IgnoreReason::AlreadyDownloaded => "already downloaded".into(),
            IgnoreReason::OutOfScope(h) => format!("{h} is not in the detection scope"),
        }
    }
}

pub fn decide(url: &str, settings: &Settings, already_downloaded: bool) -> Decision {
    let normalized = normalize_url(url);
    if settings
        .ignored_urls
        .iter()
        .any(|u| normalize_url(u) == normalized)
    {
        return Decision::Ignore(IgnoreReason::IgnoredUrl);
    }

    let host = host_of(url);
    if host.is_empty() {
        return Decision::Ignore(IgnoreReason::OutOfScope(url.to_string()));
    }

    if let Some(rule) = settings
        .ignored_domains
        .iter()
        .find(|r| domain_matches(&host, r))
    {
        return Decision::Ignore(IgnoreReason::IgnoredDomain(rule.clone()));
    }

    if already_downloaded && settings.skip_already_downloaded {
        return Decision::Ignore(IgnoreReason::AlreadyDownloaded);
    }

    let in_scope = match settings.detection_scope {
        DetectionScope::AllUrls => true,
        DetectionScope::KnownSitesOnly => {
            is_known_site(&host)
                || settings
                    .allowed_domains
                    .iter()
                    .any(|r| domain_matches(&host, r))
        }
        DetectionScope::AllowListOnly => settings
            .allowed_domains
            .iter()
            .any(|r| domain_matches(&host, r)),
    };
    if !in_scope {
        return Decision::Ignore(IgnoreReason::OutOfScope(host.clone()));
    }

    if settings
        .auto_domains
        .iter()
        .any(|r| domain_matches(&host, r))
    {
        return Decision::AutoDownload;
    }

    match settings.default_action {
        DefaultAction::Prompt => Decision::Prompt,
        DefaultAction::AutoDownload => Decision::AutoDownload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::default()
    }

    #[test]
    fn known_site_prompts_by_default() {
        assert_eq!(
            decide("https://www.youtube.com/watch?v=1", &settings(), false),
            Decision::Prompt
        );
    }

    #[test]
    fn unknown_site_is_out_of_scope_by_default() {
        assert!(matches!(
            decide("https://example.com/video", &settings(), false),
            Decision::Ignore(IgnoreReason::OutOfScope(_))
        ));
    }

    #[test]
    fn ignored_domain_wins_over_everything() {
        let mut s = settings();
        s.default_action = DefaultAction::AutoDownload;
        s.ignored_domains = vec!["youtube.com".into()];
        assert!(matches!(
            decide("https://www.youtube.com/watch?v=1", &s, false),
            Decision::Ignore(IgnoreReason::IgnoredDomain(_))
        ));
    }

    #[test]
    fn auto_domain_downloads_even_in_prompt_mode() {
        let mut s = settings();
        s.auto_domains = vec!["vimeo.com".into()];
        assert_eq!(
            decide("https://vimeo.com/123", &s, false),
            Decision::AutoDownload
        );
    }

    #[test]
    fn global_auto_mode() {
        let mut s = settings();
        s.default_action = DefaultAction::AutoDownload;
        assert_eq!(
            decide("https://twitch.tv/videos/1", &s, false),
            Decision::AutoDownload
        );
    }

    #[test]
    fn already_downloaded_is_skipped_only_when_enabled() {
        let s = settings();
        assert_eq!(
            decide("https://youtu.be/abc", &s, true),
            Decision::Ignore(IgnoreReason::AlreadyDownloaded)
        );
        let mut s2 = settings();
        s2.skip_already_downloaded = false;
        assert_eq!(decide("https://youtu.be/abc", &s2, true), Decision::Prompt);
    }

    #[test]
    fn allow_list_only_scope() {
        let mut s = settings();
        s.detection_scope = DetectionScope::AllowListOnly;
        s.allowed_domains = vec!["myvideos.example".into()];
        assert_eq!(
            decide("https://myvideos.example/watch/1", &s, false),
            Decision::Prompt
        );
        assert!(matches!(
            decide("https://www.youtube.com/watch?v=1", &s, false),
            Decision::Ignore(IgnoreReason::OutOfScope(_))
        ));
    }

    #[test]
    fn all_urls_scope_accepts_unknown_hosts() {
        let mut s = settings();
        s.detection_scope = DetectionScope::AllUrls;
        assert_eq!(
            decide("https://obscure-site.example/v/9", &s, false),
            Decision::Prompt
        );
    }

    #[test]
    fn domain_preset_pin_matches_subdomains() {
        let mut s = settings();
        s.domain_presets = vec![crate::settings::DomainPreset {
            domain: "youtube.com".into(),
            preset: Preset::P720,
        }];
        assert_eq!(
            preset_for_url("https://music.youtube.com/watch?v=1", &s),
            Some(Preset::P720)
        );
        assert_eq!(preset_for_url("https://vimeo.com/123", &s), None);
        assert_eq!(preset_for_url("not a url", &s), None);
    }

    #[test]
    fn ignored_specific_url() {
        let mut s = settings();
        s.ignored_urls = vec!["https://youtu.be/never".into()];
        assert_eq!(
            decide("https://youtu.be/never#frag", &s, false),
            Decision::Ignore(IgnoreReason::IgnoredUrl)
        );
    }
}
