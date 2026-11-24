//! Inspect local audio and create a tagged copy without modifying the source.
//! Lyrics lookup is deliberately not included; users supply their own text.
use crate::process::{self, CancellationToken, ProcessLimits, ProcessSpec};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub lyrics: String,
    /// True when ffprobe reports any known lyrics-like tag, including an empty one.
    pub lyrics_present: bool,
}

/// Locate ffprobe next to the selected ffmpeg, then on PATH.
pub fn locate_ffprobe(ffmpeg: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    ffmpeg
        .parent()
        .map(|dir| dir.join(name))
        .filter(|p| p.is_file())
        .or_else(|| crate::ytdlp::find_on_path(name))
}

fn local_input(input: &Path) -> Result<PathBuf, String> {
    let input = input
        .canonicalize()
        .map_err(|e| format!("Cannot open input: {e}"))?;
    if !input.is_file() {
        return Err("Input must be a local file".into());
    }
    let extension = input.extension().and_then(|s| s.to_str()).unwrap_or("");
    if !["mp3", "m4a", "flac", "ogg", "opus"]
        .iter()
        .any(|s| extension.eq_ignore_ascii_case(s))
    {
        return Err("Supported audio files: MP3, M4A, FLAC, OGG and Opus".into());
    }
    Ok(input)
}

/// Inspect audio with bounded output and a cancellable owned process.
pub fn inspect(
    ffprobe: &Path,
    input: &Path,
    token: &CancellationToken,
) -> Result<AudioTags, String> {
    let input = local_input(input)?;
    let spec = ProcessSpec::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type:stream_tags:format_tags",
            "-of",
            "json",
            "-i",
        ])
        .arg(input);
    let output = process::run_capture(
        &spec,
        ProcessLimits {
            timeout: std::time::Duration::from_secs(30),
            stdout_bytes: 2 * 1024 * 1024,
            stderr_bytes: 64 * 1024,
            line_bytes: 2 * 1024 * 1024,
        },
        token,
    )
    .map_err(|e| e.to_string())?;
    parse_probe(&output.stdout)
}

fn parse_probe(json: &[u8]) -> Result<AudioTags, String> {
    let value: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| format!("Invalid metadata response: {e}"))?;
    let streams = value
        .get("streams")
        .and_then(|v| v.as_array())
        .ok_or("Metadata response has no streams")?;
    if !streams
        .iter()
        .any(|stream| stream.get("codec_type").and_then(|v| v.as_str()) == Some("audio"))
    {
        return Err("File has no audio stream".into());
    }
    let format = value
        .get("format")
        .and_then(|v| v.as_object())
        .ok_or("Metadata response has no format")?;
    fn apply_tags(
        result: &mut AudioTags,
        raw_tags: &serde_json::Value,
        common: bool,
    ) -> Result<(), String> {
        let tags = raw_tags.as_object().ok_or("Metadata tags are malformed")?;
        for (key, value) in tags {
            let key = key.to_ascii_lowercase();
            let text = value.as_str().unwrap_or("");
            match key.as_str() {
                "title" if common => result.title = text.to_owned(),
                "artist" if common => result.artist = text.to_owned(),
                "album" if common => result.album = text.to_owned(),
                _ if key.contains("lyrics")
                    || key.starts_with("uslt")
                    || key.starts_with("sylt") =>
                {
                    result.lyrics_present = true;
                    if result.lyrics.is_empty() {
                        result.lyrics = text.to_owned();
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    let mut result = AudioTags::default();
    let mut first = true;
    for stream in streams
        .iter()
        .filter(|stream| stream.get("codec_type").and_then(|v| v.as_str()) == Some("audio"))
    {
        if let Some(tags) = stream.get("tags") {
            apply_tags(&mut result, tags, first)?;
        }
        first = false;
    }
    if let Some(tags) = format.get("tags") {
        apply_tags(&mut result, tags, true)?;
    }
    Ok(result)
}

/// Suggest a sibling filename without changing or reserving it.
pub fn suggested_output(input: &Path) -> Option<PathBuf> {
    let mut name = input.file_stem()?.to_os_string();
    name.push(".tagged.");
    name.push(input.extension()?);
    Some(input.with_file_name(name))
}

/// Construct a non-overwriting tagged copy. Existing lyrics reported by the
/// fresh probe are never replaced; unsupported metadata may not survive a remux.
pub fn build_tagged_copy(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    edits: &AudioTags,
    existing: &AudioTags,
) -> Result<ProcessSpec, String> {
    let input = local_input(input)?;
    let name = output.file_name().ok_or("Choose an output file")?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| format!("Cannot open output folder: {e}"))?;
    let output = parent.join(name);
    if std::fs::symlink_metadata(&output).is_ok() {
        return Err("Output already exists; choose another name".into());
    }
    if !input
        .extension()
        .and_then(|v| v.to_str())
        .zip(output.extension().and_then(|v| v.to_str()))
        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
    {
        return Err("Output must use the same extension as the input".into());
    }
    // Vorbis/Opus comments are stream metadata, not format metadata.
    let ext = input.extension().and_then(|v| v.to_str()).unwrap_or("");
    let metadata_flag = if ext.eq_ignore_ascii_case("ogg") || ext.eq_ignore_ascii_case("opus") {
        "-metadata:s:a:0"
    } else {
        "-metadata"
    };
    let added_lyrics = if existing.lyrics_present {
        ""
    } else {
        edits.lyrics.as_str()
    };
    let fields = [
        edits.title.as_str(),
        edits.artist.as_str(),
        edits.album.as_str(),
        added_lyrics,
    ];
    if fields.iter().any(|v| v.contains('\0')) {
        return Err("Metadata cannot contain a NUL character".into());
    }
    if fields
        .iter()
        .map(|v| v.encode_utf16().count())
        .sum::<usize>()
        > 12_000
    {
        return Err("Metadata is too long; use at most 12,000 UTF-16 units total".into());
    }
    let mut spec = ProcessSpec::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-n", "-i"])
        .arg(input)
        .args(["-map", "0", "-map_metadata", "0", "-c", "copy"]);
    for (name, value) in [
        ("title", &edits.title),
        ("artist", &edits.artist),
        ("album", &edits.album),
    ] {
        spec = spec.arg(metadata_flag).arg(format!("{name}={value}"));
    }
    if !added_lyrics.is_empty() {
        spec = spec
            .arg(metadata_flag)
            .arg(format!("lyrics={added_lyrics}"));
    }
    Ok(spec.arg(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_detects_audio_and_lyrics_case_insensitively() {
        let tags = parse_probe(br#"{"streams":[{"codec_type":"audio"}],"format":{"tags":{"TITLE":"A","ArTiSt":"B","UNSYNCEDLYRICS":"old"}}}"#).unwrap();
        assert_eq!(tags.title, "A");
        assert_eq!(tags.artist, "B");
        assert!(tags.lyrics_present);
        assert_eq!(tags.lyrics, "old");
        assert!(parse_probe(br#"{"streams":[{"codec_type":"video"}],"format":{}}"#).is_err());
        assert!(parse_probe(b"{} ").is_err());
        let stream = parse_probe(br#"{"streams":[{"codec_type":"audio","tags":{"TITLE":"Stream title","LYRICS":"old"}}],"format":{}}"#).unwrap();
        assert_eq!(stream.title, "Stream title");
        assert!(stream.lyrics_present);
        assert!(
            parse_probe(br#"{"streams":[{"codec_type":"audio","tags":[]}],"format":{}}"#).is_err()
        );
    }
    #[test]
    fn copy_never_overwrites_and_preserves_reported_lyrics() {
        let dir = std::env::temp_dir().join(format!("yttraydl-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("track & name.mp3");
        let output = dir.join("copy.mp3");
        std::fs::write(&input, b"original").unwrap();
        let edits = AudioTags {
            title: "New title".into(),
            lyrics: "new lyrics".into(),
            ..Default::default()
        };
        let existing = AudioTags {
            lyrics_present: true,
            ..Default::default()
        };
        let spec =
            build_tagged_copy(Path::new("ffmpeg"), &input, &output, &edits, &existing).unwrap();
        assert!(spec.args.iter().any(|v| v == "-n"));
        assert!(!spec.args.iter().any(|v| v == "lyrics=new lyrics"));
        let spec = build_tagged_copy(
            Path::new("ffmpeg"),
            &input,
            &output,
            &edits,
            &AudioTags::default(),
        )
        .unwrap();
        assert!(spec.args.iter().any(|v| v == "lyrics=new lyrics"));
        assert!(build_tagged_copy(Path::new("ffmpeg"), &input, &input, &edits, &existing).is_err());
        assert!(build_tagged_copy(
            Path::new("ffmpeg"),
            &input,
            &dir.join("copy.flac"),
            &edits,
            &existing
        )
        .is_err());
        std::fs::write(&output, b"keep").unwrap();
        assert!(
            build_tagged_copy(Path::new("ffmpeg"), &input, &output, &edits, &existing).is_err()
        );
        assert_eq!(std::fs::read(&input).unwrap(), b"original");
        assert_eq!(std::fs::read(&output).unwrap(), b"keep");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
