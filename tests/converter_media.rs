//! Opt-in end-to-end ffmpeg coverage. Set YTTRAYDL_TEST_FFMPEG to an
//! executable with libx264 to exercise the real process owner and encoder.
use std::path::PathBuf;
use yttraydl::converter::{
    build_batch_with_trim, run_batch, run_batch_with_progress, BatchProgress, TrimRange,
    VideoProfile,
};
use yttraydl::process::{run_capture, CancellationToken, ProcessLimits, ProcessSpec};

#[test]
fn ffmpeg_batch_trims_two_files() {
    let Some(ffmpeg) = std::env::var_os("YTTRAYDL_TEST_FFMPEG").map(PathBuf::from) else {
        eprintln!("Skipping ffmpeg integration: YTTRAYDL_TEST_FFMPEG is not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("yttraydl-media-convert-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("sample one.mkv");
    let second = dir.join("sample two.mkv");
    let token = CancellationToken::default();
    let fixture = ProcessSpec::new(&ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=size=64x64:rate=10",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-n",
        ])
        .arg(&first);
    run_capture(&fixture, ProcessLimits::default(), &token).unwrap();
    std::fs::copy(&first, &second).unwrap();
    let trim = TrimRange::parse("0.4", "1.4").unwrap();
    let specs =
        build_batch_with_trim(&ffmpeg, &[first, second], &dir, VideoProfile::Mp4, trim).unwrap();
    let mut progress = Vec::new();
    run_batch_with_progress(&specs, &token, |event| progress.push(event)).unwrap();
    assert!(progress.contains(&BatchProgress::Started { index: 1, total: 2 }));
    assert!(progress.contains(&BatchProgress::Started { index: 2, total: 2 }));
    for index in 1..=2 {
        assert!(progress.iter().any(|event| matches!(event, BatchProgress::EncodedTime { index: value, total: 2, .. } if *value == index)));
    }
    for file in ["sample one.converted.mp4", "sample two.converted.mp4"] {
        let decoded = ProcessSpec::new(&ffmpeg)
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(dir.join(file))
            .args(["-f", "null", "-", "-progress", "pipe:1"]);
        let output = run_capture(&decoded, ProcessLimits::default(), &token).unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        let duration = text
            .lines()
            .rev()
            .find_map(|line| line.strip_prefix("out_time_us=")?.parse::<u64>().ok())
            .unwrap();
        assert!((900_000..=1_100_000).contains(&duration), "{duration}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ffmpeg_crops_then_resizes_without_upscaling() {
    use yttraydl::converter::{
        build_conversion_with_options, ConversionOptions, CropRect, ResizePreset,
    };
    let Some(ffmpeg) = std::env::var_os("YTTRAYDL_TEST_FFMPEG").map(PathBuf::from) else {
        eprintln!("Skipping ffmpeg integration: YTTRAYDL_TEST_FFMPEG is not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("yttraydl-media-effects-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.mkv");
    let token = CancellationToken::default();
    let fixture = ProcessSpec::new(&ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=size=960x540:rate=2",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-n",
        ])
        .arg(&input);
    run_capture(&fixture, ProcessLimits::default(), &token).unwrap();
    for (name, crop, dimensions) in [
        (
            "cropped.mp4",
            CropRect::parse("800", "400", "10", "10").unwrap(),
            "s:800x400",
        ),
        ("scaled.mp4", None, "s:854x480"),
    ] {
        let output = dir.join(name);
        let options = ConversionOptions {
            crop,
            resize: ResizePreset::P480,
            ..Default::default()
        };
        let spec =
            build_conversion_with_options(&ffmpeg, &input, &output, VideoProfile::Mp4, options)
                .unwrap();
        run_batch(&[spec], &token, |_, _| ()).unwrap();
        let inspect = ProcessSpec::new(&ffmpeg)
            .args(["-nostdin", "-hide_banner", "-i"])
            .arg(output)
            .args(["-vf", "showinfo", "-frames:v", "1", "-f", "null", "-"]);
        let result = run_capture(&inspect, ProcessLimits::default(), &token).unwrap();
        let info = String::from_utf8_lossy(&result.stderr);
        assert!(info.contains(dimensions), "expected {dimensions}: {info}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// Additional opt-in: these encoders may be absent or slow on some builds.
#[test]
fn ffmpeg_modern_codec_profiles_encode() {
    use yttraydl::converter::{build_conversion_with_options, ConversionOptions, EncodingQuality};
    if std::env::var_os("YTTRAYDL_TEST_FFMPEG_ADVANCED").is_none() {
        eprintln!("Skipping modern encoders: YTTRAYDL_TEST_FFMPEG_ADVANCED is not set");
        return;
    }
    let ffmpeg =
        PathBuf::from(std::env::var_os("YTTRAYDL_TEST_FFMPEG").expect("set YTTRAYDL_TEST_FFMPEG"));
    let dir = std::env::temp_dir().join(format!("yttraydl-media-codecs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.mkv");
    let token = CancellationToken::default();
    let fixture = ProcessSpec::new(&ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=size=64x64:rate=2",
            "-t",
            "0.5",
            "-c:v",
            "libx264",
            "-n",
        ])
        .arg(&input);
    run_capture(&fixture, ProcessLimits::default(), &token).unwrap();
    for (index, profile) in [
        VideoProfile::Mp4Hevc,
        VideoProfile::MkvHevc,
        VideoProfile::Mp4Av1,
        VideoProfile::WebmAv1,
    ]
    .into_iter()
    .enumerate()
    {
        let output = dir.join(format!("output-{index}.{}", profile.extension()));
        let options = ConversionOptions {
            quality: EncodingQuality::Balanced,
            ..Default::default()
        };
        let spec =
            build_conversion_with_options(&ffmpeg, &input, &output, profile, options).unwrap();
        run_batch(&[spec], &token, |_, _| ()).unwrap();
        assert!(output.metadata().unwrap().len() > 100);
        let inspect = ProcessSpec::new(&ffmpeg)
            .args(["-nostdin", "-hide_banner", "-i"])
            .arg(&output)
            .args(["-frames:v", "1", "-f", "null", "-"]);
        let decoded = run_capture(&inspect, ProcessLimits::default(), &token).unwrap();
        let info = String::from_utf8_lossy(&decoded.stderr);
        let codec = if matches!(profile, VideoProfile::Mp4Hevc | VideoProfile::MkvHevc) {
            "Video: hevc"
        } else {
            "Video: av1"
        };
        assert!(info.contains(codec), "expected {codec}: {info}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ffmpeg_rotates_and_can_remove_audio() {
    use yttraydl::converter::{build_conversion_with_options, ConversionOptions, Rotation};
    let Some(ffmpeg) = std::env::var_os("YTTRAYDL_TEST_FFMPEG").map(PathBuf::from) else {
        eprintln!("Skipping ffmpeg integration: YTTRAYDL_TEST_FFMPEG is not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("yttraydl-media-rotate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.mkv");
    let output = dir.join("rotated.mp4");
    let token = CancellationToken::default();
    let fixture = ProcessSpec::new(&ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=size=96x64:rate=5",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=1",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-n",
        ])
        .arg(&input);
    run_capture(&fixture, ProcessLimits::default(), &token).unwrap();
    let spec = build_conversion_with_options(
        &ffmpeg,
        &input,
        &output,
        VideoProfile::Mp4,
        ConversionOptions {
            rotation: Rotation::Clockwise,
            mute_audio: true,
            ..Default::default()
        },
    )
    .unwrap();
    run_batch(&[spec], &token, |_, _| ()).unwrap();
    let inspect = ProcessSpec::new(&ffmpeg)
        .args(["-nostdin", "-hide_banner", "-i"])
        .arg(&output)
        .args(["-vf", "showinfo", "-frames:v", "1", "-f", "null", "-"]);
    let result = run_capture(&inspect, ProcessLimits::default(), &token).unwrap();
    let info = String::from_utf8_lossy(&result.stderr);
    assert!(info.contains("s:64x96"), "{info}");
    assert!(!info.contains("Audio:"), "{info}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ffmpeg_extracts_audio_profiles_from_video() {
    use yttraydl::converter::{build_conversion_with_options, AudioBitrate, ConversionOptions};
    let Some(ffmpeg) = std::env::var_os("YTTRAYDL_TEST_FFMPEG").map(PathBuf::from) else {
        eprintln!("Skipping ffmpeg integration: YTTRAYDL_TEST_FFMPEG is not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("yttraydl-media-audio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("source.mkv");
    let token = CancellationToken::default();
    let fixture = ProcessSpec::new(&ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=size=64x64:rate=5",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=1",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-n",
        ])
        .arg(&input);
    run_capture(&fixture, ProcessLimits::default(), &token).unwrap();
    for (profile, codec) in [
        (VideoProfile::Mp3Audio, "mp3"),
        (VideoProfile::M4aAudio, "aac"),
        (VideoProfile::FlacAudio, "flac"),
        (VideoProfile::OpusAudio, "opus"),
    ] {
        let output = dir.join(format!("output.{}", profile.extension()));
        let options = ConversionOptions {
            audio_bitrate: AudioBitrate::K192,
            trim: TrimRange::parse("0.1", "0.8").unwrap(),
            ..Default::default()
        };
        let spec =
            build_conversion_with_options(&ffmpeg, &input, &output, profile, options).unwrap();
        run_batch(&[spec], &token, |_, _| ()).unwrap();
        let inspect = ProcessSpec::new(&ffmpeg)
            .args(["-nostdin", "-hide_banner", "-i"])
            .arg(&output)
            .args(["-f", "null", "-"]);
        let result = run_capture(&inspect, ProcessLimits::default(), &token).unwrap();
        let info = String::from_utf8_lossy(&result.stderr);
        assert!(info.contains(&format!("Audio: {codec}")), "{info}");
        assert!(!info.contains("Video:"), "{info}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
