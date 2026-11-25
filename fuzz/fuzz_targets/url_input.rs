#![no_main]
use libfuzzer_sys::fuzz_target;
use yttraydl::url_detect::{extract_urls, host_of, is_http_url, is_known_site, normalize_url};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = is_http_url(&text);
    let _ = host_of(&text);
    let _ = normalize_url(&text);
    for url in extract_urls(&text) {
        let _ = is_http_url(&url);
        let _ = is_known_site(&host_of(&url));
        let _ = normalize_url(&url);
    }
});
