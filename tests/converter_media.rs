//! Opt-in end-to-end ffmpeg coverage. Set YTTRAYDL_TEST_FFMPEG to an
//! executable with libx264 to exercise the real process owner and encoder.
use std::path::PathBuf;
use yttraydl::converter::{build_batch_with_trim, run_batch, TrimRange, VideoProfile};
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
    run_batch(&specs, &token, |index, total| progress.push((index, total))).unwrap();
    assert_eq!(progress, [(1, 2), (2, 2)]);
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
