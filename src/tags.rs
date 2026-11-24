//! Inspect local audio and create a tagged copy without modifying the source.
use crate::process::{self, CancellationToken, ProcessLimits, ProcessSpec};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track: String,
    pub date: String,
    pub genre: String,
    pub lyrics: String,
    pub artwork: Option<Artwork>,
    /// All attached image stream indices, including unsupported codecs.
    pub attached_picture_indices: Vec<u32>,
    /// Other video streams or incomplete stream data prevent safe replacement.
    pub artwork_replacement_unsafe: bool,
    /// True when ffprobe reports any known lyrics-like tag, including an empty one.
    pub lyrics_present: bool,
}

/// First supported embedded JPEG or PNG image stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artwork {
    pub stream_index: u32,
    pub extension: &'static str,
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
            "stream=index,codec_type,codec_name:stream_disposition=attached_pic:stream_tags:format_tags",
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
                "album_artist" if common => result.album_artist = text.to_owned(),
                "track" if common => result.track = text.to_owned(),
                "date" if common => result.date = text.to_owned(),
                "genre" if common => result.genre = text.to_owned(),
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
    let artwork = streams.iter().find_map(|stream| {
        if stream.get("codec_type")?.as_str()? != "video"
            || stream.get("disposition")?.get("attached_pic")?.as_i64()? != 1
        {
            return None;
        }
        let extension = match stream.get("codec_name")?.as_str()? {
            "mjpeg" => "jpg",
            "png" => "png",
            _ => return None,
        };
        let stream_index = u32::try_from(stream.get("index")?.as_u64()?).ok()?;
        Some(Artwork {
            stream_index,
            extension,
        })
    });
    let mut result = AudioTags {
        artwork,
        ..Default::default()
    };
    for stream in streams
        .iter()
        .filter(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("video"))
    {
        let index = stream
            .get("index")
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok());
        let attached = stream
            .get("disposition")
            .and_then(|v| v.get("attached_pic"))
            .and_then(|v| v.as_i64())
            == Some(1);
        if let (true, Some(index)) = (attached, index) {
            result.attached_picture_indices.push(index);
        } else {
            result.artwork_replacement_unsafe = true;
        }
    }
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
    build_tagged_copy_with_artwork(ffmpeg, input, output, edits, existing, None)
}

/// Export a tagged copy and optionally replace embedded artwork with a local
/// JPEG/PNG. Only MP3/M4A/FLAC are supported for replacement. The source and
/// existing destinations are not modified; ffmpeg may not preserve every tag.
pub fn build_tagged_copy_with_artwork(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    edits: &AudioTags,
    existing: &AudioTags,
    replacement: Option<&Path>,
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
    let replacement = if let Some(path) = replacement {
        if !["mp3", "m4a", "flac"]
            .iter()
            .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        {
            return Err("Artwork replacement is supported for MP3, M4A and FLAC only".into());
        }
        if existing.artwork_replacement_unsafe {
            return Err("Cannot safely replace artwork in a file with other video streams".into());
        }
        let path = path
            .canonicalize()
            .map_err(|e| format!("Cannot open artwork: {e}"))?;
        let metadata = path
            .metadata()
            .map_err(|e| format!("Cannot inspect artwork: {e}"))?;
        if !metadata.is_file() || metadata.len() > 10 * 1024 * 1024 || metadata.len() < 8 {
            return Err("Artwork must be a JPEG or PNG file no larger than 10 MiB".into());
        }
        let mut file =
            std::fs::File::open(&path).map_err(|e| format!("Cannot read artwork: {e}"))?;
        let mut header = [0u8; 8];
        std::io::Read::read_exact(&mut file, &mut header)
            .map_err(|e| format!("Cannot read artwork: {e}"))?;
        if !header.starts_with(&[0xff, 0xd8, 0xff]) && header != *b"\x89PNG\r\n\x1a\n" {
            return Err("Artwork must be a JPEG or PNG image".into());
        }
        Some(path)
    } else {
        None
    };
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
        edits.album_artist.as_str(),
        edits.track.as_str(),
        edits.date.as_str(),
        edits.genre.as_str(),
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
        .arg(input);
    if let Some(path) = replacement {
        spec = spec.arg("-i").arg(path).args(["-map", "0"]);
        for index in &existing.attached_picture_indices {
            spec = spec.arg("-map").arg(format!("-0:{index}"));
        }
        spec = spec.args(["-map", "1:v:0", "-disposition:v:0", "attached_pic"]);
    } else {
        spec = spec.args(["-map", "0"]);
    }
    spec = spec.args(["-map_metadata", "0", "-c", "copy"]);
    for (name, value) in [
        ("title", &edits.title),
        ("artist", &edits.artist),
        ("album", &edits.album),
        ("album_artist", &edits.album_artist),
        ("track", &edits.track),
        ("date", &edits.date),
        ("genre", &edits.genre),
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

/// Extract an embedded JPEG or PNG to a new image without transcoding it.
/// The caller must probe the input immediately before constructing this command.
pub fn build_artwork_export(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    artwork: &Artwork,
) -> Result<ProcessSpec, String> {
    let input = local_input(input)?;
    let name = output.file_name().ok_or("Choose an image output file")?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| format!("Cannot open output folder: {e}"))?;
    let output = parent.join(name);
    if std::fs::symlink_metadata(&output).is_ok() {
        return Err("Image output already exists".into());
    }
    let ext = output.extension().and_then(|v| v.to_str()).unwrap_or("");
    let valid_ext = match artwork.extension {
        "jpg" => ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg"),
        "png" => ext.eq_ignore_ascii_case("png"),
        _ => false,
    };
    if !valid_ext {
        return Err(format!(
            "Image output must match the embedded .{} format",
            artwork.extension
        ));
    }
    Ok(ProcessSpec::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-n", "-i"])
        .arg(input)
        .arg("-map")
        .arg(format!("0:{}", artwork.stream_index))
        .args([
            "-frames:v",
            "1",
            "-c:v",
            "copy",
            "-f",
            "image2",
            "-update",
            "1",
        ])
        .arg(output))
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
        let cover = parse_probe(br#"{"streams":[{"codec_type":"audio"},{"codec_type":"video","index":3,"codec_name":"png","disposition":{"attached_pic":1}}],"format":{}}"#).unwrap();
        assert_eq!(
            cover.artwork,
            Some(Artwork {
                stream_index: 3,
                extension: "png"
            })
        );
        let not_cover = parse_probe(br#"{"streams":[{"codec_type":"audio"},{"codec_type":"video","index":3,"codec_name":"png","disposition":{"attached_pic":0}}],"format":{}}"#).unwrap();
        assert_eq!(not_cover.artwork, None);
        let stream = parse_probe(br#"{"streams":[{"codec_type":"audio","tags":{"TITLE":"Stream title","LYRICS":"old"}}],"format":{}}"#).unwrap();
        assert_eq!(stream.title, "Stream title");
        assert!(stream.lyrics_present);
        assert!(
            parse_probe(br#"{"streams":[{"codec_type":"audio","tags":[]}],"format":{}}"#).is_err()
        );
    }
    #[test]
    fn replacement_rejects_unsafe_and_invalid_inputs() {
        let dir =
            std::env::temp_dir().join(format!("yttraydl-art-validation-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("source.mp3");
        let output = dir.join("copy.mp3");
        let image = dir.join("cover.png");
        std::fs::write(&input, b"source").unwrap();
        std::fs::write(&image, b"not an image").unwrap();
        let tags = AudioTags::default();
        assert!(build_tagged_copy_with_artwork(
            Path::new("ffmpeg"),
            &input,
            &output,
            &tags,
            &tags,
            Some(&image)
        )
        .is_err());
        std::fs::write(&image, b"\x89PNG\r\n\x1a\n").unwrap();
        let unsafe_tags = AudioTags {
            artwork_replacement_unsafe: true,
            ..Default::default()
        };
        assert!(build_tagged_copy_with_artwork(
            Path::new("ffmpeg"),
            &input,
            &output,
            &tags,
            &unsafe_tags,
            Some(&image)
        )
        .is_err());
        let ogg = dir.join("source.ogg");
        std::fs::write(&ogg, b"source").unwrap();
        assert!(build_tagged_copy_with_artwork(
            Path::new("ffmpeg"),
            &ogg,
            &dir.join("copy.ogg"),
            &tags,
            &tags,
            Some(&image)
        )
        .is_err());
        let spec = build_tagged_copy_with_artwork(
            Path::new("ffmpeg"),
            &input,
            &output,
            &tags,
            &tags,
            Some(&image),
        )
        .unwrap();
        assert!(spec.args.iter().any(|arg| arg == "-n"));
        assert!(spec.args.iter().any(|arg| arg == "attached_pic"));
        std::fs::remove_dir_all(dir).unwrap();
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
