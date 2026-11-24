//! Local video conversion: validate paths and construct a shell-free ffmpeg invocation.
use crate::process::{self, CancellationToken, OutputStream, ProcessLimits, ProcessSpec};
use crate::settings::Settings;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoProfile {
    #[default]
    Mp4,
    Mkv,
    Webm,
    Mp4Hevc,
    MkvHevc,
    Mp4Av1,
    WebmAv1,
}

impl VideoProfile {
    pub const ALL: [Self; 7] = [
        Self::Mp4,
        Self::Mkv,
        Self::Webm,
        Self::Mp4Hevc,
        Self::MkvHevc,
        Self::Mp4Av1,
        Self::WebmAv1,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4 (H.264 + AAC)",
            Self::Mkv => "MKV (H.264 + AAC)",
            Self::Webm => "WebM (VP9 + Opus)",
            Self::Mp4Hevc => "MP4 (HEVC + AAC)",
            Self::MkvHevc => "MKV (HEVC + AAC)",
            Self::Mp4Av1 => "MP4 (AV1 + AAC)",
            Self::WebmAv1 => "WebM (AV1 + Opus)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 | Self::Mp4Hevc | Self::Mp4Av1 => "mp4",
            Self::Mkv | Self::MkvHevc => "mkv",
            Self::Webm | Self::WebmAv1 => "webm",
        }
    }
}

/// Video encoding quality; encoder default preserves previous behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncodingQuality {
    #[default]
    EncoderDefault,
    High,
    Balanced,
    Smaller,
}
impl EncodingQuality {
    pub const ALL: [Self; 4] = [
        Self::EncoderDefault,
        Self::High,
        Self::Balanced,
        Self::Smaller,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::EncoderDefault => "Encoder default",
            Self::High => "Higher quality (larger files)",
            Self::Balanced => "Balanced",
            Self::Smaller => "Smaller files",
        }
    }
    fn crf(self, profile: VideoProfile) -> Option<&'static str> {
        let is_modern = matches!(
            profile,
            VideoProfile::Webm | VideoProfile::Mp4Av1 | VideoProfile::WebmAv1
        );
        match (self, is_modern) {
            (Self::EncoderDefault, _) => None,
            (Self::High, false) => Some("18"),
            (Self::Balanced, false) => Some("23"),
            (Self::Smaller, false) => Some("28"),
            (Self::High, true) => Some("24"),
            (Self::Balanced, true) => Some("32"),
            (Self::Smaller, true) => Some("40"),
        }
    }
}

/// Optional accurate re-encoding trim, in milliseconds. The end is exclusive
/// and must follow the start. An absent end keeps the rest of the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrimRange {
    start_ms: u64,
    end_ms: Option<u64>,
}

impl TrimRange {
    /// Parse non-negative decimal seconds with up to millisecond precision.
    /// Blank start means zero; blank end means no explicit end.
    pub fn parse(start: &str, end: &str) -> Result<Self, String> {
        let start_ms = if start.trim().is_empty() {
            0
        } else {
            parse_seconds(start)?
        };
        let end_ms = if end.trim().is_empty() {
            None
        } else {
            Some(parse_seconds(end)?)
        };
        if end_ms.is_some_and(|end| end <= start_ms) {
            return Err("Trim end must be later than the start".into());
        }
        Ok(Self { start_ms, end_ms })
    }
}

fn parse_seconds(value: &str) -> Result<u64, String> {
    let value = value.trim();
    let (seconds, fraction) = value.split_once('.').unwrap_or((value, ""));
    if seconds.is_empty()
        || !seconds.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 3
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (value.contains('.') && fraction.is_empty())
    {
        return Err("Trim times must be non-negative seconds (up to 3 decimals)".into());
    }
    let seconds: u64 = seconds.parse().map_err(|_| "Trim time is too large")?;
    let millis = if fraction.is_empty() {
        0
    } else {
        let digits = fraction.len() as u32;
        let fraction: u64 = fraction.parse().map_err(|_| "Invalid trim time")?;
        fraction * 10_u64.pow(3 - digits)
    };
    seconds
        .checked_mul(1000)
        .and_then(|s| s.checked_add(millis))
        .ok_or_else(|| "Trim time is too large".into())
}

/// A numeric crop rectangle in source pixels, before any resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
}

impl CropRect {
    /// Blank fields disable cropping; otherwise width and height are required.
    /// Blank offsets mean zero. Expressions and negative values are rejected.
    pub fn parse(width: &str, height: &str, x: &str, y: &str) -> Result<Option<Self>, String> {
        if [width, height, x, y].iter().all(|v| v.trim().is_empty()) {
            return Ok(None);
        }
        fn number(value: &str, name: &str, default_zero: bool) -> Result<u32, String> {
            let value = value.trim();
            if default_zero && value.is_empty() {
                return Ok(0);
            }
            if value.is_empty() || !value.bytes().all(|v| v.is_ascii_digit()) {
                return Err(format!("Crop {name} must be a non-negative whole number"));
            }
            value
                .parse()
                .map_err(|_| format!("Crop {name} is too large"))
        }
        let width = number(width, "width", false)?;
        let height = number(height, "height", false)?;
        if width == 0 || height == 0 {
            return Err("Crop width and height must be greater than zero".into());
        }
        Ok(Some(Self {
            width,
            height,
            x: number(x, "x", true)?,
            y: number(y, "y", true)?,
        }))
    }
}

/// Fit inside a bounding box without upscaling, preserving aspect ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizePreset {
    #[default]
    Original,
    P480,
    P720,
    P1080,
    P2160,
}
impl ResizePreset {
    pub const ALL: [Self; 5] = [
        Self::Original,
        Self::P480,
        Self::P720,
        Self::P1080,
        Self::P2160,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original size",
            Self::P480 => "Fit within 854 × 480",
            Self::P720 => "Fit within 1280 × 720",
            Self::P1080 => "Fit within 1920 × 1080",
            Self::P2160 => "Fit within 3840 × 2160",
        }
    }
    fn dimensions(self) -> Option<(u32, u32)> {
        match self {
            Self::Original => None,
            Self::P480 => Some((854, 480)),
            Self::P720 => Some((1280, 720)),
            Self::P1080 => Some((1920, 1080)),
            Self::P2160 => Some((3840, 2160)),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConversionOptions {
    pub trim: TrimRange,
    pub crop: Option<CropRect>,
    pub resize: ResizePreset,
    pub quality: EncodingQuality,
}

fn video_filters(options: ConversionOptions) -> Option<String> {
    let mut filters = Vec::new();
    if let Some(crop) = options.crop {
        filters.push(format!(
            "crop={}:{}:{}:{}:exact=1",
            crop.width, crop.height, crop.x, crop.y
        ));
    }
    if let Some((width, height)) = options.resize.dimensions() {
        // Quotes are parsed by ffmpeg's filter parser, not a shell. The min
        // expressions prevent enlarging small inputs, including portrait video.
        filters.push(format!("scale=w='min(iw,{width})':h='min(ih,{height})':force_original_aspect_ratio=decrease:force_divisible_by=2"));
    }
    (!filters.is_empty()).then(|| filters.join(","))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchEdit {
    Remove(usize),
    MoveUp(usize),
    MoveDown(usize),
}

/// Reorder or remove a pending input. Invalid indices are harmless.
pub fn edit_batch(inputs: &mut Vec<PathBuf>, edit: BatchEdit) -> bool {
    match edit {
        BatchEdit::Remove(index) if index < inputs.len() => {
            inputs.remove(index);
            true
        }
        BatchEdit::MoveUp(index) if index > 0 && index < inputs.len() => {
            inputs.swap(index, index - 1);
            true
        }
        BatchEdit::MoveDown(index) if index < inputs.len().saturating_sub(1) => {
            inputs.swap(index, index + 1);
            true
        }
        _ => false,
    }
}

fn seconds_arg(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// Resolve ffmpeg in the same order used by the downloader. An explicit but
/// invalid path is an error rather than silently running another binary.
pub fn locate_ffmpeg(settings: &Settings, exe_dir: Option<&Path>) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if !settings.ffmpeg_path.as_os_str().is_empty() {
        return settings
            .ffmpeg_path
            .is_file()
            .then(|| settings.ffmpeg_path.clone());
    }
    if let Some(path) = exe_dir
        .map(|dir| dir.join(file))
        .filter(|path| path.is_file())
    {
        return Some(path);
    }
    let managed = crate::tools::managed_ffmpeg_path();
    if managed.is_file() {
        return Some(managed);
    }
    crate::ytdlp::find_on_path(file)
}

/// Only local existing input files and a new output in an existing directory
/// are accepted. ffmpeg is also instructed not to overwrite existing files.
pub fn build_conversion(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    profile: VideoProfile,
) -> Result<ProcessSpec, String> {
    build_conversion_with_trim(ffmpeg, input, output, profile, TrimRange::default())
}

/// Build a conversion with an optional, re-encoded trim range.
pub fn build_conversion_with_trim(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    profile: VideoProfile,
    trim: TrimRange,
) -> Result<ProcessSpec, String> {
    build_conversion_with_options(
        ffmpeg,
        input,
        output,
        profile,
        ConversionOptions {
            trim,
            ..Default::default()
        },
    )
}

/// Build an invocation with validated trim, crop and resize options.
pub fn build_conversion_with_options(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    profile: VideoProfile,
    options: ConversionOptions,
) -> Result<ProcessSpec, String> {
    let trim = options.trim;
    let input = input
        .canonicalize()
        .map_err(|e| format!("Cannot open input: {e}"))?;
    if !input.is_file() {
        return Err("Input must be a local file".into());
    }
    let name = output.file_name().ok_or("Choose an output file")?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent
        .canonicalize()
        .map_err(|e| format!("Cannot open output folder: {e}"))?;
    let output = parent.join(name);
    if std::fs::symlink_metadata(&output).is_ok() {
        return Err("Output already exists; choose another name".into());
    }
    if !output
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(profile.extension()))
    {
        return Err(format!(
            "Output filename must end in .{}",
            profile.extension()
        ));
    }
    let (video, audio) = match profile {
        VideoProfile::Mp4 | VideoProfile::Mkv => ("libx264", "aac"),
        VideoProfile::Webm => ("libvpx-vp9", "libopus"),
        VideoProfile::Mp4Hevc | VideoProfile::MkvHevc => ("libx265", "aac"),
        VideoProfile::Mp4Av1 => ("libaom-av1", "aac"),
        VideoProfile::WebmAv1 => ("libaom-av1", "libopus"),
    };
    let mut spec = ProcessSpec::new(ffmpeg)
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-progress",
            "pipe:1",
            "-nostats",
            "-n",
            "-i",
        ])
        .arg(input);
    if trim.start_ms > 0 {
        spec = spec.arg("-ss").arg(seconds_arg(trim.start_ms));
    }
    if let Some(end_ms) = trim.end_ms {
        spec = spec.arg("-t").arg(seconds_arg(end_ms - trim.start_ms));
    }
    if let Some(filters) = video_filters(options) {
        spec = spec.arg("-vf").arg(filters);
    }
    spec = spec.args([
        "-map", "0:v:0", "-map", "0:a:0?", "-c:v", video, "-c:a", audio,
    ]);
    if let Some(crf) = options.quality.crf(profile) {
        spec = spec.args(["-crf", crf]);
        if matches!(
            profile,
            VideoProfile::Webm | VideoProfile::Mp4Av1 | VideoProfile::WebmAv1
        ) {
            spec = spec.args(["-b:v", "0"]);
        }
    }
    // MP4 HEVC needs this tag for broader player compatibility.
    if profile == VideoProfile::Mp4Hevc {
        spec = spec.args(["-tag:v", "hvc1"]);
    }
    Ok(spec.arg(output))
}

/// Build a sequential batch only after every item passes validation. Outputs
/// are named `<input stem>.converted.<extension>` in the chosen folder. An
/// existing destination or two inputs mapping to the same name rejects the
/// entire batch before a process starts.
pub fn build_batch(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    output_dir: &Path,
    profile: VideoProfile,
) -> Result<Vec<ProcessSpec>, String> {
    build_batch_with_trim(ffmpeg, inputs, output_dir, profile, TrimRange::default())
}

/// Preflight a batch with the same trim applied to every input.
pub fn build_batch_with_trim(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    output_dir: &Path,
    profile: VideoProfile,
    trim: TrimRange,
) -> Result<Vec<ProcessSpec>, String> {
    build_batch_with_options(
        ffmpeg,
        inputs,
        output_dir,
        profile,
        ConversionOptions {
            trim,
            ..Default::default()
        },
    )
}

/// Preflight a batch with the same validated options for every item.
pub fn build_batch_with_options(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    output_dir: &Path,
    profile: VideoProfile,
    options: ConversionOptions,
) -> Result<Vec<ProcessSpec>, String> {
    if inputs.is_empty() {
        return Err("Choose at least one input file".into());
    }
    let folder = output_dir
        .canonicalize()
        .map_err(|e| format!("Cannot open output folder: {e}"))?;
    if !folder.is_dir() {
        return Err("Output folder must be a directory".into());
    }
    let mut outputs: Vec<PathBuf> = Vec::new();
    let mut specs = Vec::with_capacity(inputs.len());
    for input in inputs {
        let mut name = input
            .file_stem()
            .ok_or("Input must have a filename")?
            .to_os_string();
        name.push(".converted.");
        name.push(profile.extension());
        let output = folder.join(name);
        // Windows filenames are case-insensitive in normal installations.
        // Conservatively reject case variants on all platforms.
        let key = output.to_string_lossy().to_lowercase();
        if outputs
            .iter()
            .any(|existing| existing.to_string_lossy().to_lowercase() == key)
        {
            return Err(format!(
                "Multiple inputs map to the same output: {}",
                output.display()
            ));
        }
        specs.push(build_conversion_with_options(
            ffmpeg, input, &output, profile, options,
        )?);
        outputs.push(output);
    }
    Ok(specs)
}

/// A conversion's position in a batch and its encoded media time. Elapsed
/// media time is not a percentage and may differ from wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchProgress {
    Started {
        index: usize,
        total: usize,
    },
    EncodedTime {
        index: usize,
        total: usize,
        microseconds: u64,
    },
}

/// Incremental parser for ffmpeg's machine-readable stdout. Process callbacks
/// receive arbitrary chunks, not lines; malformed or oversized lines are ignored.
#[derive(Default)]
struct ProgressParser {
    line: Vec<u8>,
    dropping: bool,
}
impl ProgressParser {
    fn push(&mut self, bytes: &[u8], mut emit: impl FnMut(u64)) {
        for &byte in bytes {
            if byte == b'\n' {
                if !self.dropping {
                    if let Ok(line) = std::str::from_utf8(&self.line) {
                        if let Some(value) =
                            line.trim_end_matches('\r').strip_prefix("out_time_us=")
                        {
                            if let Ok(micros) = value.parse::<u64>() {
                                emit(micros);
                            }
                        }
                    }
                }
                self.line.clear();
                self.dropping = false;
            } else if !self.dropping {
                if self.line.len() >= 256 {
                    self.line.clear();
                    self.dropping = true;
                } else {
                    self.line.push(byte);
                }
            }
        }
    }
}

/// Execute a batch sequentially; stop on the first failure or cancellation.
/// Completed outputs are kept. Reports starts and throttled ffmpeg progress.
pub fn run_batch_with_progress(
    specs: &[ProcessSpec],
    token: &CancellationToken,
    mut progress: impl FnMut(BatchProgress),
) -> Result<(), String> {
    if specs.is_empty() {
        return Err("No conversions to run".into());
    }
    for (position, spec) in specs.iter().enumerate() {
        let index = position + 1;
        let total = specs.len();
        if token.is_cancelled() {
            return Err(format!("cancelled before file {index} of {total}"));
        }
        progress(BatchProgress::Started { index, total });
        let mut parser = ProgressParser::default();
        let mut last_report = None;
        let limits = ProcessLimits {
            timeout: std::time::Duration::from_secs(6 * 60 * 60),
            stdout_bytes: 8192,
            stderr_bytes: 64 * 1024,
            line_bytes: 4096,
        };
        let run = process::OwnedProcess::spawn(spec).and_then(|owner| {
            owner.stream(limits, token, |stream, chunk| {
                if stream == OutputStream::Stdout {
                    parser.push(chunk, |microseconds| {
                        let now = std::time::Instant::now();
                        if last_report.is_none_or(|last: std::time::Instant| {
                            now.duration_since(last) >= std::time::Duration::from_millis(250)
                        }) {
                            last_report = Some(now);
                            progress(BatchProgress::EncodedTime {
                                index,
                                total,
                                microseconds,
                            });
                        }
                    });
                }
            })
        });
        run.map_err(|error| format!("file {index} of {total}: {error}"))?;
    }
    Ok(())
}

/// Compatibility wrapper for callers interested only in batch item starts.
pub fn run_batch(
    specs: &[ProcessSpec],
    token: &CancellationToken,
    mut progress: impl FnMut(usize, usize),
) -> Result<(), String> {
    run_batch_with_progress(specs, token, |event| {
        if let BatchProgress::Started { index, total } = event {
            progress(index, total);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_parser_handles_chunks_invalid_lines_and_bounds() {
        let mut parser = ProgressParser::default();
        let mut times = Vec::new();
        parser.push(b"out_time_", |v| times.push(v));
        parser.push(b"us=12345\r\nout_time_us=-1\n", |v| times.push(v));
        parser.push(&vec![b'a'; 300], |v| times.push(v));
        parser.push(b"out_time_us=88\nout_time_us=9000000\n", |v| times.push(v));
        assert_eq!(times, [12_345, 9_000_000]);
    }

    #[test]
    fn all_profiles_map_to_encoders_and_quality_settings() {
        let dir = std::env::temp_dir().join(format!("yttraydl-codec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input.mkv");
        std::fs::write(&input, b"input").unwrap();
        for (index, profile) in VideoProfile::ALL.into_iter().enumerate() {
            let output = dir.join(format!("result-{index}.{}", profile.extension()));
            let options = ConversionOptions {
                quality: EncodingQuality::High,
                ..Default::default()
            };
            let spec = build_conversion_with_options(
                Path::new("ffmpeg"),
                &input,
                &output,
                profile,
                options,
            )
            .unwrap();
            let video = match profile {
                VideoProfile::Mp4 | VideoProfile::Mkv => "libx264",
                VideoProfile::Webm => "libvpx-vp9",
                VideoProfile::Mp4Hevc | VideoProfile::MkvHevc => "libx265",
                VideoProfile::Mp4Av1 | VideoProfile::WebmAv1 => "libaom-av1",
            };
            assert!(spec
                .args
                .windows(2)
                .any(|pair| pair[0] == "-c:v" && pair[1] == video));
            assert!(spec
                .args
                .windows(2)
                .any(|pair| pair[0] == "-crf" && pair[1] == options.quality.crf(profile).unwrap()));
            assert!(spec
                .args
                .windows(2)
                .any(|pair| pair[0] == "-progress" && pair[1] == "pipe:1"));
            let uses_zero_bitrate = matches!(
                profile,
                VideoProfile::Webm | VideoProfile::Mp4Av1 | VideoProfile::WebmAv1
            );
            assert_eq!(
                spec.args
                    .windows(2)
                    .any(|pair| pair[0] == "-b:v" && pair[1] == "0"),
                uses_zero_bitrate
            );
        }
        let default = build_conversion(
            Path::new("ffmpeg"),
            &input,
            &dir.join("default.mp4"),
            VideoProfile::Mp4,
        )
        .unwrap();
        assert!(!default.args.iter().any(|arg| arg == "-crf"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn crop_validation_and_filter_order_are_safe() {
        assert_eq!(CropRect::parse("", "", "", "").unwrap(), None);
        for values in [
            ("", "10", "", ""),
            ("0", "10", "", ""),
            ("10", "0", "", ""),
            ("10", "10", "-1", ""),
            ("10:evil", "10", "", ""),
            ("4294967296", "10", "", ""),
        ] {
            assert!(CropRect::parse(values.0, values.1, values.2, values.3).is_err());
        }
        let crop = CropRect::parse("640", "360", "10", "20").unwrap();
        let options = ConversionOptions {
            crop,
            resize: ResizePreset::P720,
            ..Default::default()
        };
        let filters = video_filters(options).unwrap();
        assert!(filters.starts_with("crop=640:360:10:20:exact=1,scale="));
        assert!(filters.contains("min(iw,1280)"));
        assert!(video_filters(ConversionOptions::default()).is_none());
        assert_eq!(
            video_filters(ConversionOptions {
                crop,
                ..Default::default()
            })
            .unwrap(),
            "crop=640:360:10:20:exact=1"
        );
    }

    #[test]
    fn batch_queue_edits_respect_bounds_and_order() {
        let mut items: Vec<PathBuf> = ["a", "b", "c"].into_iter().map(PathBuf::from).collect();
        assert!(!edit_batch(&mut items, BatchEdit::MoveUp(0)));
        assert!(!edit_batch(&mut items, BatchEdit::MoveDown(2)));
        assert!(!edit_batch(&mut items, BatchEdit::Remove(3)));
        assert!(edit_batch(&mut items, BatchEdit::MoveDown(0)));
        assert_eq!(
            items,
            [PathBuf::from("b"), PathBuf::from("a"), PathBuf::from("c")]
        );
        assert!(edit_batch(&mut items, BatchEdit::MoveUp(2)));
        assert!(edit_batch(&mut items, BatchEdit::Remove(1)));
        assert_eq!(items, [PathBuf::from("b"), PathBuf::from("a")]);
    }

    #[test]
    fn batch_applies_crop_and_resize_to_each_item() {
        let dir = std::env::temp_dir().join(format!("yttraydl-crop-batch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let inputs = [dir.join("a.mov"), dir.join("b.mov")];
        for path in &inputs {
            std::fs::write(path, b"input").unwrap();
        }
        let options = ConversionOptions {
            trim: TrimRange::parse("0.5", "1.5").unwrap(),
            crop: CropRect::parse("640", "360", "0", "0").unwrap(),
            resize: ResizePreset::P480,
            ..Default::default()
        };
        let specs = build_batch_with_options(
            Path::new("ffmpeg"),
            &inputs,
            &dir,
            VideoProfile::Mp4,
            options,
        )
        .unwrap();
        for spec in specs {
            assert!(spec.args.windows(2).any(|p| p[0] == "-vf"
                && p[1]
                    .to_string_lossy()
                    .starts_with("crop=640:360:0:0:exact=1,scale=")));
            assert!(spec
                .args
                .windows(2)
                .any(|p| p[0] == "-t" && p[1] == "1.000"));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn trim_range_validates_decimal_seconds() {
        assert_eq!(TrimRange::parse("", "").unwrap(), TrimRange::default());
        assert_eq!(
            TrimRange::parse("1.25", "3.001").unwrap(),
            TrimRange {
                start_ms: 1250,
                end_ms: Some(3001)
            }
        );
        assert_eq!(TrimRange::parse("", "0.010").unwrap().end_ms, Some(10));
        for (start, end) in [
            ("-1", ""),
            ("NaN", ""),
            ("1.0001", ""),
            ("1.", ""),
            ("3", "3"),
            ("4", "3"),
            ("1", "0"),
            ("18446744073709551615", ""),
        ] {
            assert!(TrimRange::parse(start, end).is_err(), "{start:?}, {end:?}");
        }
    }

    #[test]
    fn trim_args_are_accurate_and_applied_to_each_batch_item() {
        let dir = std::env::temp_dir().join(format!("yttraydl-trim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.mov");
        let b = dir.join("b.mov");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let trim = TrimRange::parse("1.125", "2.5").unwrap();
        let specs =
            build_batch_with_trim(Path::new("ffmpeg"), &[a, b], &dir, VideoProfile::Mp4, trim)
                .unwrap();
        for spec in specs {
            assert!(spec
                .args
                .windows(2)
                .any(|p| p[0] == "-ss" && p[1] == "1.125"));
            assert!(spec
                .args
                .windows(2)
                .any(|p| p[0] == "-t" && p[1] == "1.375"));
            let input_pos = spec.args.iter().position(|p| p == "-i").unwrap();
            let seek_pos = spec.args.iter().position(|p| p == "-ss").unwrap();
            assert!(seek_pos > input_pos);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn paths_are_arguments_and_existing_files_are_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("yttraydl-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("source & clip.mov");
        let output = dir.join("result.mp4");
        std::fs::write(&input, b"input").unwrap();
        let spec =
            build_conversion(Path::new("ffmpeg"), &input, &output, VideoProfile::Mp4).unwrap();
        assert!(spec.args.iter().any(|arg| arg == "-n"));
        assert!(spec
            .args
            .iter()
            .any(|arg| arg.as_os_str() == input.canonicalize().unwrap().as_os_str()));
        assert!(spec.args.iter().any(
            |arg| arg.as_os_str() == dir.canonicalize().unwrap().join("result.mp4").as_os_str()
        ));
        assert!(build_conversion(Path::new("ffmpeg"), &input, &input, VideoProfile::Mp4).is_err());
        assert!(
            build_conversion(Path::new("ffmpeg"), &input, &output, VideoProfile::Webm).is_err()
        );
        std::fs::write(&output, b"do not replace").unwrap();
        assert!(build_conversion(Path::new("ffmpeg"), &input, &output, VideoProfile::Mp4).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"do not replace");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn batch_preflights_conflicts_and_builds_separate_outputs() {
        let dir = std::env::temp_dir().join(format!("yttraydl-batch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("first.mov");
        let b = dir.join("second.mkv");
        let collision = dir.join("FIRST.avi");
        for path in [&a, &b, &collision] {
            std::fs::write(path, b"input").unwrap();
        }
        let specs = build_batch(
            Path::new("ffmpeg"),
            &[a.clone(), b.clone()],
            &dir,
            VideoProfile::Mp4,
        )
        .unwrap();
        assert_eq!(specs.len(), 2);
        assert!(specs[0]
            .args
            .last()
            .unwrap()
            .to_string_lossy()
            .ends_with("first.converted.mp4"));
        assert!(specs[1]
            .args
            .last()
            .unwrap()
            .to_string_lossy()
            .ends_with("second.converted.mp4"));
        assert!(build_batch(
            Path::new("ffmpeg"),
            &[a.clone(), collision],
            &dir,
            VideoProfile::Mp4
        )
        .is_err());
        std::fs::write(dir.join("second.converted.mp4"), b"keep").unwrap();
        assert!(build_batch(Path::new("ffmpeg"), &[a, b], &dir, VideoProfile::Mp4).is_err());
        assert_eq!(
            std::fs::read(dir.join("second.converted.mp4")).unwrap(),
            b"keep"
        );
        assert!(build_batch(Path::new("ffmpeg"), &[], &dir, VideoProfile::Mp4).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_input_is_rejected() {
        assert!(build_conversion(
            Path::new("ffmpeg"),
            Path::new("missing-file"),
            Path::new("out.mp4"),
            VideoProfile::Mp4
        )
        .is_err());
    }
}
