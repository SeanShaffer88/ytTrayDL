//! Opt-in: YTTRAYDL_TEST_FFMPEG and YTTRAYDL_TEST_FFPROBE must name executables.
use std::path::PathBuf;
use yttraydl::process::{run_capture, CancellationToken, ProcessLimits, ProcessSpec};
use yttraydl::tags::{build_tagged_copy, inspect, AudioTags};

#[test]
fn audio_tagged_copies_preserve_source_and_existing_lyrics() {
    let (Some(ffmpeg), Some(ffprobe)) = (
        std::env::var_os("YTTRAYDL_TEST_FFMPEG").map(PathBuf::from),
        std::env::var_os("YTTRAYDL_TEST_FFPROBE").map(PathBuf::from),
    ) else {
        eprintln!("Skipping audio tag integration: ffmpeg/ffprobe not configured");
        return;
    };
    let dir = std::env::temp_dir().join(format!("yttraydl-tags-media-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let token = CancellationToken::default();
    for (ext, codec) in [
        ("mp3", "libmp3lame"),
        ("m4a", "aac"),
        ("flac", "flac"),
        ("ogg", "libvorbis"),
        ("opus", "libopus"),
    ] {
        for existing_lyrics in [false, true] {
            let input = dir.join(format!("input-{existing_lyrics}.{ext}"));
            let output = dir.join(format!("output-{existing_lyrics}.{ext}"));
            let mut fixture = ProcessSpec::new(&ffmpeg).args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:duration=0.1",
                "-c:a",
                codec,
                "-metadata",
                "title=Original",
                "-metadata",
                "artist=Artist",
                "-n",
            ]);
            if existing_lyrics {
                fixture = fixture.args(["-metadata", "lyrics=Old lyrics"]);
            }
            fixture = fixture.arg(&input);
            run_capture(&fixture, ProcessLimits::default(), &token).unwrap();
            let before = std::fs::read(&input).unwrap();
            let existing = inspect(&ffprobe, &input, &token).unwrap();
            assert_eq!(existing.title, "Original", "{ext}");
            assert_eq!(existing.lyrics_present, existing_lyrics, "{ext}");
            let edit = AudioTags {
                title: "New title".into(),
                artist: "New artist".into(),
                album: "New album".into(),
                lyrics: "New lyrics".into(),
                ..Default::default()
            };
            let spec = build_tagged_copy(&ffmpeg, &input, &output, &edit, &existing).unwrap();
            run_capture(&spec, ProcessLimits::default(), &token).unwrap();
            assert_eq!(std::fs::read(&input).unwrap(), before, "{ext}");
            let result = inspect(&ffprobe, &output, &token).unwrap();
            assert_eq!(result.title, "New title", "{ext}");
            assert_eq!(result.artist, "New artist", "{ext}");
            assert_eq!(result.album, "New album", "{ext}");
            assert_eq!(
                result.lyrics,
                if existing_lyrics {
                    "Old lyrics"
                } else {
                    "New lyrics"
                },
                "{ext}"
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
