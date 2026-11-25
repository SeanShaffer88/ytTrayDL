//! Inspect per-domain decisions locally without saving settings or networking.
use yttraydl::rules::{decide, preset_for_url, Decision};
use yttraydl::settings::{DefaultAction, DomainPreset, Preset, Settings};

fn main() {
    let mut settings = Settings {
        default_action: DefaultAction::Prompt,
        auto_domains: vec!["youtube.com".into()],
        domain_presets: vec![DomainPreset {
            domain: "youtube.com".into(),
            preset: Preset::P720,
        }],
        ..Settings::default()
    };
    let url = "https://www.youtube.com/watch?v=example";
    assert_eq!(decide(url, &settings, false), Decision::AutoDownload);
    assert_eq!(preset_for_url(url, &settings), Some(Preset::P720));
    settings.auto_domains.clear();
    assert_eq!(decide(url, &settings, false), Decision::Prompt);
    settings.default_action = DefaultAction::AutoDownload;
    assert_eq!(decide(url, &settings, false), Decision::AutoDownload);
    println!("Per-domain quality, prompt and auto-download decisions passed.");
}
