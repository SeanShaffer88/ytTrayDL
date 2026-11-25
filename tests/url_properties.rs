//! Deterministic generated-input properties, no external services or timing.
use std::time::Duration;
use yttraydl::{dedupe::SeenCache, url_detect, ytdlp};
use yttraydl_property_check::Generator;

#[test]
fn generated_http_schemes_domains_and_playlists() {
    let mut generator = Generator::new(0x4f6c_6f63_616c);
    for case in 0..1024 {
        let id = generator.identifier();
        let list = generator.identifier();
        let url = format!("https://www.youtube.com/watch?v={id}&list={list}");
        assert!(url_detect::is_http_url(&url), "case {case}");
        assert!(
            url_detect::is_known_site(&url_detect::host_of(&url)),
            "case {case}"
        );
        assert_eq!(url_detect::extract_urls(&url), [url.clone()], "case {case}");
        for prefix in ["httpx://", "http:/", "https:/", "ftp://"] {
            assert!(
                !url_detect::is_http_url(&format!("{prefix}youtube.com/{id}")),
                "case {case}"
            );
        }
        assert!(!url_detect::is_known_site("youtube.com.attacker.invalid"));
    }
}

#[test]
fn generated_dedupe_and_archive_respect_video_id_case() {
    let mut generator = Generator::new(0x7365_6564);
    for case in 0..1024 {
        let id = generator.identifier();
        let url = format!("https://youtube.com/watch?v={id}");
        let equivalent = format!("HTTPS://YOUTUBE.COM/watch?v={id}#fragment");
        let mut seen = SeenCache::new(Duration::from_secs(60));
        assert!(seen.mark(&url), "case {case}");
        assert!(!seen.mark(&equivalent), "case {case}");
        let archive = format!("YouTube {id}\n");
        assert!(
            ytdlp::archive_contains(&archive, "youtube", &id),
            "case {case}"
        );
        assert!(
            !ytdlp::archive_contains(&archive, "youtube", &format!("{id}x")),
            "case {case}"
        );
        let upper = format!("{id}A");
        let lower = format!("{id}a");
        assert!(
            !ytdlp::archive_contains(&format!("youtube {upper}\n"), "YOUTUBE", &lower),
            "case {case}"
        );
    }
}
