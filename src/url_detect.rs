//! URL extraction from clipboard text and "is this supported?"
//! classification.
//!
//! Clipboard text is arbitrary: a bare link, a sentence containing a
//! link, a Markdown link, several links. Extraction is a small hand
//! parser (no regex dependency): find `http://` / `https://`, consume
//! URL characters, then trim trailing punctuation that prose adds
//! around a link, honouring balanced parentheses (Wikipedia-style
//! URLs and many share links contain them).

/// Extract every http(s) URL from `text`, in order, de-duplicated,
/// with the original casing preserved.
pub fn extract_urls(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if starts_url(text, i) {
            let start = i;
            let mut end = i;
            while end < bytes.len() && is_url_char(bytes[end]) {
                end += 1;
            }
            let raw = &text[start..end];
            let cleaned = trim_url(raw);
            if cleaned.len() >= 10 && !out.iter().any(|u| u == cleaned) {
                out.push(cleaned.to_string());
            }
            i = end.max(i + 1);
        } else {
            i += text[i..].chars().next().map_or(1, |c| c.len_utf8());
        }
    }
    out
}

fn starts_url(text: &str, i: usize) -> bool {
    let rest = &text[i..];
    rest.starts_with("https://") || rest.starts_with("http://")
}

fn is_url_char(b: u8) -> bool {
    // URLs in clipboard text end at whitespace or common delimiters
    // used by markup/chat clients. Everything else ASCII-graphic is
    // allowed; non-ASCII bytes belong to percent-decoded-in-place
    // IRIs some apps copy, so allow them too.
    !(b <= b' ' || matches!(b, b'"' | b'<' | b'>' | b'`' | 0x7f))
}

/// Trim prose punctuation from the end of a URL: sentence-ending
/// `. , ; : ! ?`, unmatched closing brackets, and stray quotes.
fn trim_url(raw: &str) -> &str {
    let mut s = raw;
    loop {
        let Some(last) = s.chars().last() else {
            return s;
        };
        let strip = match last {
            '.' | ',' | ';' | '!' | '?' | '\'' | '»' | '…' => true,
            ')' => s.matches(')').count() > s.matches('(').count(),
            ']' => s.matches(']').count() > s.matches('[').count(),
            '}' => s.matches('}').count() > s.matches('{').count(),
            ':' => true, // a trailing colon is never significant here
            _ => false,
        };
        if !strip {
            return s;
        }
        s = &s[..s.len() - last.len_utf8()];
    }
}

/// The lowercase host of a URL, without port or userinfo.
/// Returns an empty string when no host can be found.
pub fn host_of(url: &str) -> String {
    let Some(after_scheme) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.split("://").nth(1))
    else {
        return String::new();
    };
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(rest) = authority.strip_prefix('[') {
        // IPv6 literal — take up to ']'
        rest.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host.to_lowercase()
}

/// Canonical form used for dedupe / ignored-URL comparisons:
/// lowercase scheme+host, default ports removed, fragment dropped,
/// trailing slash on a bare root dropped. Query strings are kept —
/// for video sites the query *is* the video id.
pub fn normalize_url(url: &str) -> String {
    let mut u = url.trim().to_string();
    if let Some(pos) = u.find('#') {
        u.truncate(pos);
    }
    // Scheme check on the raw bytes (perf): this used to lowercase
    // the whole URL just to test the prefix. Only ASCII case can
    // vary in a scheme, so a case-insensitive byte compare is
    // equivalent — and slicing at 7/8 is safe once those bytes are
    // known to be ASCII.
    let bytes = u.as_bytes();
    let (scheme, rest) = if bytes.len() >= 8 && bytes[..8].eq_ignore_ascii_case(b"https://") {
        ("https", &u[8..])
    } else if bytes.len() >= 7 && bytes[..7].eq_ignore_ascii_case(b"http://") {
        ("http", &u[7..])
    } else {
        return u;
    };
    let (authority, tail) = match rest.find(['/', '?']) {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, ""),
    };
    let mut auth = authority.to_lowercase();
    if (scheme == "https" && auth.ends_with(":443")) || (scheme == "http" && auth.ends_with(":80"))
    {
        let cut = auth.rfind(':').unwrap();
        auth.truncate(cut);
    }
    let tail = if tail == "/" { "" } else { tail };
    format!("{scheme}://{auth}{tail}")
}

/// Hosts for the built-in known-sites list: the most-used yt-dlp
/// extractors. yt-dlp supports far more sites than this list —
/// `DetectionScope::AllUrls` exists for those; this list is the
/// quiet default that keeps ordinary web links from triggering
/// popups. Kept as bare registrable domains; subdomains match.
pub const KNOWN_SITES: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "youtube-nocookie.com",
    "vimeo.com",
    "dailymotion.com",
    "twitch.tv",
    "tiktok.com",
    "instagram.com",
    "facebook.com",
    "fb.watch",
    "fb.gg",
    "twitter.com",
    "x.com",
    "reddit.com",
    "soundcloud.com",
    "bandcamp.com",
    "bilibili.com",
    "nicovideo.jp",
    "niconico.com",
    "rumble.com",
    "odysee.com",
    "streamable.com",
    "vevo.com",
    "mixcloud.com",
    "audiomack.com",
    "patreon.com",
    "nebula.tv",
    "curiositystream.com",
    "crunchyroll.com",
    "funimation.com",
    "vk.com",
    "ok.ru",
    "rutube.ru",
    "likee.video",
    "kwai.com",
    "snapchat.com",
    "pinterest.com",
    "tumblr.com",
    "flickr.com",
    "archive.org",
    "medal.tv",
    "kick.com",
    "loco.com",
    "trovo.live",
    "dlive.tv",
    "peertube.tv",
    "mastodon.social",
    "bsky.app",
    "threads.net",
    "linkedin.com",
    "ted.com",
    "khanacademy.org",
    "coursera.org",
    "udemy.com",
    "bbc.co.uk",
    "bbc.com",
    "cnn.com",
    "nytimes.com",
    "theguardian.com",
    "washingtonpost.com",
    "npr.org",
    "pbs.org",
    "arte.tv",
    "zdf.de",
    "ardmediathek.de",
    "9gag.com",
    "imgur.com",
    "gfycat.com",
    "tenor.com",
];

/// Does `host` equal `domain` or sit on a subdomain of it?
/// Allocation-free (perf): the naive form builds a `".{domain}"`
/// string per check, and the rules engine checks every detected
/// link against the whole known-sites list plus the user's rules.
pub(crate) fn ends_with_domain(host: &str, domain: &str) -> bool {
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}

pub fn is_known_site(host: &str) -> bool {
    KNOWN_SITES.iter().any(|site| ends_with_domain(host, site))
}

/// Allocation-free canonical equality: true exactly when
/// `normalize_url(a) == normalize_url(b)`, computed on slices.
/// The rules engine's ignored-URL check runs this on every
/// detected link, where the allocating form cost several heap
/// strings per stored rule. Inputs containing non-ASCII fall
/// back to the allocating comparison, so Unicode lowercasing
/// of authorities behaves identically by construction.
pub fn normalized_url_eq(a: &str, b: &str) -> bool {
    if !a.is_ascii() || !b.is_ascii() {
        return normalize_url(a) == normalize_url(b);
    }
    canonical_eq(&canonical_parts(a), &canonical_parts(b))
}

/// The pieces `normalize_url` compares, borrowed from the input:
/// canonical scheme (when an http(s) prefix matched), the
/// authority and tail in their original case, or — with no
/// scheme — the whole trimmed, fragment-free string (`plain`),
/// which `normalize_url` returns unchanged.
struct CanonicalParts<'a> {
    scheme: Option<&'static str>,
    authority: &'a str,
    tail: &'a str,
    plain: &'a str,
}

fn canonical_parts(url: &str) -> CanonicalParts<'_> {
    let u = url.trim();
    let u = match u.find('#') {
        Some(pos) => &u[..pos],
        None => u,
    };
    let bytes = u.as_bytes();
    let (scheme, rest) = if bytes.len() >= 8 && bytes[..8].eq_ignore_ascii_case(b"https://") {
        ("https", &u[8..])
    } else if bytes.len() >= 7 && bytes[..7].eq_ignore_ascii_case(b"http://") {
        ("http", &u[7..])
    } else {
        return CanonicalParts {
            scheme: None,
            authority: "",
            tail: "",
            plain: u,
        };
    };
    let (authority, tail) = match rest.find(['/', '?']) {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, ""),
    };
    CanonicalParts {
        scheme: Some(scheme),
        authority,
        tail: if tail == "/" { "" } else { tail },
        plain: "",
    }
}

fn canonical_eq(a: &CanonicalParts, b: &CanonicalParts) -> bool {
    match (a.scheme, b.scheme) {
        (None, None) => a.plain == b.plain,
        (Some(sa), Some(sb)) => {
            sa == sb
                && strip_default_port(a.authority, sa)
                    .eq_ignore_ascii_case(strip_default_port(b.authority, sb))
                && a.tail == b.tail
        }
        // One side canonicalized to scheme form, the other did
        // not — their normalized strings cannot be equal (the
        // scheme form starts with the lowercase scheme prefix,
        // which the plain form by definition does not match even
        // case-insensitively).
        _ => false,
    }
}

/// `normalize_url` drops the scheme's default port from the
/// authority (`https://…:443`, `http://…:80`); mirror that here.
/// Stripping the exact suffix is equivalent to the original's
/// truncate-at-last-colon: when the authority ends with the
/// suffix, its colon is the last one.
fn strip_default_port<'a>(authority: &'a str, scheme: &str) -> &'a str {
    let suffix = if scheme == "https" { ":443" } else { ":80" };
    authority.strip_suffix(suffix).unwrap_or(authority)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_bare_url() {
        assert_eq!(
            extract_urls("https://www.youtube.com/watch?v=abc123"),
            vec!["https://www.youtube.com/watch?v=abc123"]
        );
    }

    #[test]
    fn extracts_from_sentence_and_strips_punctuation() {
        let urls = extract_urls("watch this (https://youtu.be/xyz789), it's great.");
        assert_eq!(urls, vec!["https://youtu.be/xyz789"]);
    }

    #[test]
    fn keeps_balanced_parentheses() {
        let urls = extract_urls("https://en.wikipedia.org/wiki/A_(b) done");
        assert_eq!(urls, vec!["https://en.wikipedia.org/wiki/A_(b)"]);
    }

    #[test]
    fn extracts_multiple_in_order_without_dupes() {
        let text = "a https://vimeo.com/1 b https://twitch.tv/x and https://vimeo.com/1 again";
        assert_eq!(
            extract_urls(text),
            vec!["https://vimeo.com/1", "https://twitch.tv/x"]
        );
    }

    #[test]
    fn ignores_non_urls() {
        assert!(extract_urls("hello world, visit youtube.com sometime").is_empty());
        assert!(extract_urls("").is_empty());
    }

    #[test]
    fn markdown_link() {
        let urls = extract_urls("[cool video](https://www.tiktok.com/@u/video/123)");
        assert_eq!(urls, vec!["https://www.tiktok.com/@u/video/123"]);
    }

    #[test]
    fn host_parsing() {
        assert_eq!(
            host_of("https://Music.YouTube.com:443/watch?v=1"),
            "music.youtube.com"
        );
        assert_eq!(host_of("https://user@x.com/post"), "x.com");
        assert_eq!(host_of("not a url"), "");
    }

    #[test]
    fn normalize_for_dedupe() {
        assert_eq!(
            normalize_url("HTTPS://YouTube.com:443/watch?v=AbC#t=10"),
            "https://youtube.com/watch?v=AbC"
        );
        assert_eq!(normalize_url("https://youtu.be/"), "https://youtu.be");
    }

    #[test]
    fn known_sites_match_subdomains() {
        assert!(is_known_site("music.youtube.com"));
        assert!(is_known_site("youtu.be"));
        assert!(!is_known_site("fakeyoutube.com"));
        assert!(!is_known_site("example.com"));
    }

    #[test]
    fn normalized_url_eq_agrees_with_normalize_url() {
        // The allocation-free equality must be a perfect twin
        // of comparing the canonical forms — every pair here is
        // checked against `normalize_url` itself as the oracle,
        // in both argument orders.
        let pairs: &[(&str, &str)] = &[
            ("https://youtu.be/abc", "https://youtu.be/abc"),
            (
                "HTTPS://YouTube.com:443/watch?v=AbC#t=10",
                "https://youtube.com/watch?v=AbC",
            ),
            ("http://example.com:80/a", "http://example.com/a"),
            ("http://example.com:443/a", "http://example.com/a"), // 443 is not http's default
            ("https://example.com:8443/a", "https://example.com/a"),
            ("https://youtu.be/", "https://youtu.be"),
            ("https://youtu.be/?x=1", "https://youtu.be?x=1"), // tail "/" only drops when bare
            ("https://youtu.be/abc", "https://youtu.be/abd"),
            ("https://youtu.be/abc", "https://youtu.be/abc?x=1"), // query is significant
            ("  https://youtu.be/abc  ", "https://youtu.be/abc"),
            ("https://user@x.com:443/post", "https://user@x.com/post"),
            ("https://[::1]:443/a", "https://[::1]/a"),
            ("not a url", "not a url"),
            ("not a url", "also not a url"),
            ("not a url", "https://youtu.be/abc"),
            ("ftp://example.com/a", "ftp://example.com/a"),
            ("HTTPS://EXAMPLE.COM/A", "https://example.com/A"), // tail case is kept
            ("https://example.com/a#frag", "https://example.com/a#other"),
        ];
        for (a, b) in pairs {
            let expected = normalize_url(a) == normalize_url(b);
            assert_eq!(normalized_url_eq(a, b), expected, "pair: {a:?} vs {b:?}");
            assert_eq!(normalized_url_eq(b, a), expected, "pair: {b:?} vs {a:?}");
        }
        // Known answers, asserted directly as well.
        assert!(normalized_url_eq(
            "HTTPS://YouTube.com:443/watch?v=AbC#t=10",
            "https://youtube.com/watch?v=AbC"
        ));
        assert!(!normalized_url_eq(
            "https://youtu.be/abc",
            "https://youtu.be/abd"
        ));
    }
}
