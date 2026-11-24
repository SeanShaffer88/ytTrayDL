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
                album_artist: "Album artist".into(),
                track: "2/10".into(),
                date: "2024".into(),
                genre: "Rock".into(),
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
            assert_eq!(result.album_artist, "Album artist", "{ext}");
            assert_eq!(result.track, "2/10", "{ext}");
            assert_eq!(result.date, "2024", "{ext}");
            assert_eq!(result.genre, "Rock", "{ext}");
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

#[test]
fn embedded_artwork_can_be_exported_without_overwriting() {
    use yttraydl::tags::build_artwork_export;
    let (Some(ffmpeg), Some(ffprobe)) = (
        std::env::var_os("YTTRAYDL_TEST_FFMPEG").map(PathBuf::from),
        std::env::var_os("YTTRAYDL_TEST_FFPROBE").map(PathBuf::from),
    ) else {
        eprintln!("Skipping artwork integration: ffmpeg/ffprobe not configured");
        return;
    };
    let dir = std::env::temp_dir().join(format!("yttraydl-art-media-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let token = CancellationToken::default();
    for (ext, codec) in [("jpg", "mjpeg"), ("png", "png")] {
        let image = dir.join(format!("cover.{ext}"));
        let input = dir.join(format!("song-{ext}.mp3"));
        let output = dir.join(format!("extracted.{ext}"));
        let fixture = ProcessSpec::new(&ffmpeg)
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=32x32",
                "-frames:v",
                "1",
                "-c:v",
                codec,
                "-threads",
                "1",
                "-update",
                "1",
                "-n",
            ])
            .arg(&image);
        run_capture(&fixture, ProcessLimits::default(), &token).unwrap();
        let fixture = ProcessSpec::new(&ffmpeg)
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=duration=0.1",
                "-i",
            ])
            .arg(&image)
            .args([
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:a",
                "libmp3lame",
                "-c:v",
                "copy",
                "-disposition:v",
                "attached_pic",
                "-n",
            ])
            .arg(&input);
        run_capture(&fixture, ProcessLimits::default(), &token).unwrap();
        let input_bytes = std::fs::read(&input).unwrap();
        let tags = inspect(&ffprobe, &input, &token).unwrap();
        let artwork = tags.artwork.unwrap();
        assert_eq!(artwork.extension, ext);
        assert!(build_artwork_export(&ffmpeg, &input, &dir.join("bad.mp3"), &artwork).is_err());
        let spec = build_artwork_export(&ffmpeg, &input, &output, &artwork).unwrap();
        run_capture(&spec, ProcessLimits::default(), &token).unwrap();
        assert_eq!(
            std::fs::read(&image).unwrap(),
            std::fs::read(&output).unwrap()
        );
        assert_eq!(std::fs::read(&input).unwrap(), input_bytes);
        assert!(build_artwork_export(&ffmpeg, &input, &output, &artwork).is_err());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replace_cover_in_tagged_copy_preserves_original_and_lyrics() {
    use yttraydl::tags::{build_artwork_export, build_tagged_copy_with_artwork};
    let (Some(ffmpeg), Some(ffprobe)) = (
        std::env::var_os("YTTRAYDL_TEST_FFMPEG").map(PathBuf::from),
        std::env::var_os("YTTRAYDL_TEST_FFPROBE").map(PathBuf::from),
    ) else {
        eprintln!("Skipping artwork integration: ffmpeg/ffprobe not configured");
        return;
    };
    let dir = std::env::temp_dir().join(format!("yttraydl-replace-art-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let token = CancellationToken::default();
    let cover = dir.join("new.png");
    let spec = ProcessSpec::new(&ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=32x32",
            "-frames:v",
            "1",
            "-c:v",
            "png",
            "-threads",
            "1",
            "-update",
            "1",
            "-n",
        ])
        .arg(&cover);
    run_capture(&spec, ProcessLimits::default(), &token).unwrap();
    let old = dir.join("old.jpg");
    let spec = ProcessSpec::new(&ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=32x32",
            "-frames:v",
            "1",
            "-c:v",
            "mjpeg",
            "-threads",
            "1",
            "-update",
            "1",
            "-n",
        ])
        .arg(&old);
    run_capture(&spec, ProcessLimits::default(), &token).unwrap();
    for (ext, encoder) in [("mp3", "libmp3lame"), ("m4a", "aac"), ("flac", "flac")] {
        for with_cover in [false, true] {
            let input = dir.join(format!("source-{with_cover}.{ext}"));
            let output = dir.join(format!("copy-{with_cover}.{ext}"));
            let extracted = dir.join(format!("extracted-{ext}-{with_cover}.png"));
            let mut fixture = ProcessSpec::new(&ffmpeg).args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=duration=0.1",
            ]);
            if with_cover {
                fixture = fixture.arg("-i").arg(&old).args([
                    "-map",
                    "0:a",
                    "-map",
                    "1:v",
                    "-c:v",
                    "copy",
                    "-disposition:v",
                    "attached_pic",
                ]);
            }
            fixture = fixture
                .args([
                    "-c:a",
                    encoder,
                    "-metadata",
                    "title=Original",
                    "-metadata",
                    "lyrics=Old lyrics",
                    "-n",
                ])
                .arg(&input);
            run_capture(&fixture, ProcessLimits::default(), &token).unwrap();
            let before = std::fs::read(&input).unwrap();
            let existing = inspect(&ffprobe, &input, &token).unwrap();
            assert_eq!(existing.artwork.is_some(), with_cover, "{ext}");
            let edits = AudioTags {
                title: "Edited".into(),
                lyrics: "Do not replace".into(),
                ..existing.clone()
            };
            let spec = build_tagged_copy_with_artwork(
                &ffmpeg,
                &input,
                &output,
                &edits,
                &existing,
                Some(&cover),
            )
            .unwrap();
            run_capture(&spec, ProcessLimits::default(), &token)
                .unwrap_or_else(|e| panic!("{ext}, cover={with_cover}: {e}"));
            assert_eq!(std::fs::read(&input).unwrap(), before);
            let result = inspect(&ffprobe, &output, &token).unwrap();
            assert_eq!(result.title, "Edited", "{ext}");
            assert_eq!(result.lyrics, "Old lyrics", "{ext}");
            assert_eq!(result.attached_picture_indices.len(), 1, "{ext}");
            let image = build_artwork_export(
                &ffmpeg,
                &output,
                &extracted,
                result.artwork.as_ref().unwrap(),
            )
            .unwrap();
            run_capture(&image, ProcessLimits::default(), &token).unwrap();
            assert_eq!(
                std::fs::read(&cover).unwrap(),
                std::fs::read(extracted).unwrap()
            );
            assert!(build_tagged_copy_with_artwork(
                &ffmpeg,
                &input,
                &output,
                &edits,
                &existing,
                Some(&cover)
            )
            .is_err());
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
