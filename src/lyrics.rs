//! Explicit online lyrics search. No request is made without a user action.
use crate::process::{self, CancellationToken, ProcessLimits, ProcessSpec};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SEARCH_URL: &str = "https://lrclib.net/api/search";
const MAX_RESPONSE: usize = 1024 * 1024;
const MAX_LYRICS: usize = 48 * 1024;
const MAX_RESULTS: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricsCandidate {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub lyrics: String,
}

fn curl_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let path = PathBuf::from(root).join("System32").join("curl.exe");
        path.is_file().then_some(path)
    }
    #[cfg(not(windows))]
    {
        crate::ytdlp::find_on_path("curl")
    }
}

/// Build a fixed-host HTTPS request; metadata is only passed as data arguments.
pub fn search_spec(curl: &Path, title: &str, artist: &str) -> Result<ProcessSpec, String> {
    let title = title.trim();
    let artist = artist.trim();
    if title.is_empty() || artist.is_empty() {
        return Err("Enter both a title and artist to search".into());
    }
    if [title, artist]
        .iter()
        .any(|s| s.len() > 512 || s.chars().any(char::is_control))
    {
        return Err(
            "Title and artist must each be under 512 bytes and contain no control characters"
                .into(),
        );
    }
    Ok(ProcessSpec::new(curl)
        .args([
            "-q",
            "--proto",
            "=https",
            "--max-redirs",
            "0",
            "--connect-timeout",
            "10",
            "--max-time",
            "25",
            "--fail",
            "--silent",
            "--show-error",
            "--get",
            "--user-agent",
            "ytTrayDL/0.8.2 (https://github.com/SeanShaffer88/ytTrayDL)",
            "--max-filesize",
            "1048576",
            "--data-urlencode",
        ])
        .arg(format!("track_name={title}"))
        .arg("--data-urlencode")
        .arg(format!("artist_name={artist}"))
        .args(["--url", SEARCH_URL]))
}

/// Parse a bounded provider response, preserving candidates for explicit review.
pub fn parse_search(bytes: &[u8]) -> Result<Vec<LyricsCandidate>, String> {
    if bytes.len() > MAX_RESPONSE {
        return Err("Lyrics response is too large".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "Invalid lyrics response")?;
    let results = value.as_array().ok_or("Invalid lyrics response")?;
    let mut candidates = Vec::new();
    for item in results.iter().take(100) {
        let Some(lyrics) = item.get("plainLyrics").and_then(|v| v.as_str()) else {
            continue;
        };
        if lyrics.trim().is_empty()
            || lyrics.len() > MAX_LYRICS
            || lyrics.encode_utf16().count() > 10_000
            || lyrics.contains('\0')
        {
            continue;
        }
        let text = |key: &str| {
            item.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .take(200)
                .collect::<String>()
        };
        let title = text("trackName");
        let artist = text("artistName");
        if title.is_empty() || artist.is_empty() {
            continue;
        }
        candidates.push(LyricsCandidate {
            title,
            artist,
            album: text("albumName"),
            lyrics: lyrics.to_owned(),
        });
        if candidates.len() == MAX_RESULTS {
            break;
        }
    }
    Ok(candidates)
}

pub fn search(
    title: &str,
    artist: &str,
    token: &CancellationToken,
) -> Result<Vec<LyricsCandidate>, String> {
    let curl = curl_path().ok_or("curl is unavailable")?;
    let spec = search_spec(&curl, title, artist)?;
    let output = process::run_capture(
        &spec,
        ProcessLimits {
            timeout: Duration::from_secs(30),
            stdout_bytes: MAX_RESPONSE,
            stderr_bytes: 16 * 1024,
            line_bytes: MAX_RESPONSE,
        },
        token,
    )
    .map_err(|e| format!("Lyrics search failed: {e}"))?;
    parse_search(&output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_is_fixed_host_and_metadata_is_data() {
        let spec = search_spec(Path::new("curl"), "A & B", "--url evil").unwrap();
        assert!(spec.args.iter().any(|a| a == "track_name=A & B"));
        assert!(spec.args.iter().any(|a| a == "artist_name=--url evil"));
        assert_eq!(spec.args.last().unwrap(), SEARCH_URL);
        assert!(spec.args.iter().any(|a| a == "=https"));
        assert!(search_spec(Path::new("curl"), "", "artist").is_err());
        assert!(search_spec(Path::new("curl"), "title\nfoo", "artist").is_err());
    }
    #[test]
    fn candidates_require_plain_lyrics_and_identifiers() {
        let value = br#"[{"trackName":"Song", "artistName":"Artist", "albumName":"Album", "plainLyrics":"first\nsecond"},{"trackName":"No words","artistName":"Artist","plainLyrics":null},{"plainLyrics":"unidentified"}]"#;
        let candidates = parse_search(value).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].lyrics, "first\nsecond");
        assert!(parse_search(b"{}").is_err());
        assert!(parse_search(&vec![b' '; MAX_RESPONSE + 1]).is_err());
    }
}
