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
    Mp3Audio,
    M4aAudio,
    FlacAudio,
    OpusAudio,
}

impl VideoProfile {
    pub const ALL: [Self; 11] = [
        Self::Mp4,
        Self::Mkv,
        Self::Webm,
        Self::Mp4Hevc,
        Self::MkvHevc,
        Self::Mp4Av1,
        Self::WebmAv1,
        Self::Mp3Audio,
        Self::M4aAudio,
        Self::FlacAudio,
        Self::OpusAudio,
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
            Self::Mp3Audio => "Audio only: MP3",
            Self::M4aAudio => "Audio only: M4A (AAC)",
            Self::FlacAudio => "Audio only: FLAC",
            Self::OpusAudio => "Audio only: Opus",
        }
    }

    pub fn is_audio_only(self) -> bool {
        matches!(
            self,
            Self::Mp3Audio | Self::M4aAudio | Self::FlacAudio | Self::OpusAudio
        )
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 | Self::Mp4Hevc | Self::Mp4Av1 => "mp4",
            Self::Mkv | Self::MkvHevc => "mkv",
            Self::Webm | Self::WebmAv1 => "webm",
            Self::Mp3Audio => "mp3",
            Self::M4aAudio => "m4a",
            Self::FlacAudio => "flac",
            Self::OpusAudio => "opus",
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

/// Trim range in milliseconds. Re-encoding is accurate; stream-copy is keyframe-limited. The end is exclusive
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

/// Explicit hardware encoders. They require a compatible GPU, driver and ffmpeg
/// build; no automatic fallback is performed if initialization fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoEncoder {
    #[default]
    Software,
    Nvenc,
    Qsv,
    Amf,
}
impl VideoEncoder {
    pub const ALL: [Self; 4] = [Self::Software, Self::Nvenc, Self::Qsv, Self::Amf];
    pub fn label(self) -> &'static str {
        match self {
            Self::Software => "Software (default)",
            Self::Nvenc => "NVIDIA NVENC (H.264 / HEVC)",
            Self::Qsv => "Intel Quick Sync (H.264 / HEVC)",
            Self::Amf => "AMD AMF (H.264 / HEVC)",
        }
    }
    fn codec(self, profile: VideoProfile) -> Result<Option<&'static str>, String> {
        if self == Self::Software {
            return Ok(None);
        }
        let hevc = match profile {
            VideoProfile::Mp4 | VideoProfile::Mkv => false,
            VideoProfile::Mp4Hevc | VideoProfile::MkvHevc => true,
            _ => {
                return Err("Hardware encoding supports only H.264 or HEVC MP4/MKV profiles".into())
            }
        };
        Ok(Some(match (self, hevc) {
            (Self::Nvenc, false) => "h264_nvenc",
            (Self::Nvenc, true) => "hevc_nvenc",
            (Self::Qsv, false) => "h264_qsv",
            (Self::Qsv, true) => "hevc_qsv",
            (Self::Amf, false) => "h264_amf",
            (Self::Amf, true) => "hevc_amf",
            (Self::Software, _) => unreachable!(),
        }))
    }
}

/// Approximate the trimmed output duration from the container duration.
/// Unknown/zero lengths and trims beyond EOF intentionally produce no estimate.
pub fn trimmed_duration_us(source_us: u64, trim: TrimRange) -> Option<u64> {
    let start = trim.start_ms.checked_mul(1000)?;
    let end = match trim.end_ms {
        Some(end) => source_us.min(end.checked_mul(1000)?),
        None => source_us,
    };
    end.checked_sub(start).filter(|&v| v > 0)
}

/// Conservative per-file estimate; 100% is reserved for successful completion.
/// ETA is withheld until at least one second and one percent of work have elapsed.
pub fn estimate_progress(
    encoded_us: u64,
    duration_us: u64,
    elapsed: std::time::Duration,
) -> Option<(u8, Option<std::time::Duration>)> {
    if duration_us == 0 {
        return None;
    }
    let fraction = (encoded_us as f64 / duration_us as f64).clamp(0.0, 1.0);
    let percent = ((fraction * 100.0) as u8).min(99);
    let remaining = if (0.01..1.0).contains(&fraction) && elapsed.as_secs() >= 1 {
        let seconds = elapsed.as_secs_f64() * (1.0 - fraction) / fraction;
        (seconds.is_finite() && seconds <= 7.0 * 24.0 * 3600.0)
            .then(|| std::time::Duration::from_secs_f64(seconds))
    } else {
        None
    };
    Some((percent, remaining))
}

/// Probe a local file for an approximate container duration. Errors, unknown
/// duration, or cancellation return None; conversion remains usable without ffprobe.
pub fn probe_duration_us(ffprobe: &Path, input: &Path, token: &CancellationToken) -> Option<u64> {
    if token.is_cancelled() {
        return None;
    }
    let input = input.canonicalize().ok()?;
    if !input.is_file() {
        return None;
    }
    let spec = ProcessSpec::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            "-i",
        ])
        .arg(input);
    let output = process::run_capture(
        &spec,
        ProcessLimits {
            timeout: std::time::Duration::from_secs(15),
            stdout_bytes: 4096,
            stderr_bytes: 8192,
            line_bytes: 4096,
        },
        token,
    )
    .ok()?;
    let value = std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()?;
    if !value.is_finite() || value <= 0.0 || value > 7.0 * 24.0 * 3600.0 {
        return None;
    }
    let micros = (value * 1_000_000.0).round() as u64;
    (micros > 0).then_some(micros)
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

/// Optional bitrate for lossy audio output; FLAC remains lossless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioBitrate {
    #[default]
    EncoderDefault,
    K128,
    K192,
    K256,
    K320,
}
impl AudioBitrate {
    pub const ALL: [Self; 5] = [
        Self::EncoderDefault,
        Self::K128,
        Self::K192,
        Self::K256,
        Self::K320,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::EncoderDefault => "Encoder default",
            Self::K128 => "128 kbps",
            Self::K192 => "192 kbps",
            Self::K256 => "256 kbps",
            Self::K320 => "320 kbps",
        }
    }
    fn argument(self) -> Option<&'static str> {
        match self {
            Self::EncoderDefault => None,
            Self::K128 => Some("128k"),
            Self::K192 => Some("192k"),
            Self::K256 => Some("256k"),
            Self::K320 => Some("320k"),
        }
    }
}

/// Rotate the decoded frame after cropping and before resizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    None,
    Clockwise,
    HalfTurn,
    CounterClockwise,
}
impl Rotation {
    pub const ALL: [Self; 4] = [
        Self::None,
        Self::Clockwise,
        Self::HalfTurn,
        Self::CounterClockwise,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No rotation",
            Self::Clockwise => "90° clockwise",
            Self::HalfTurn => "180°",
            Self::CounterClockwise => "90° counterclockwise",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConversionOptions {
    pub trim: TrimRange,
    pub crop: Option<CropRect>,
    pub resize: ResizePreset,
    pub quality: EncodingQuality,
    pub encoder: VideoEncoder,
    pub rotation: Rotation,
    pub mute_audio: bool,
    pub audio_bitrate: AudioBitrate,
}

/// Conservative preflight for a lossless trim attempt. `probe_json` is
/// bounded ffprobe JSON containing stream codec_name and codec_type fields.
/// Unknown or missing data fails closed to re-encoding. This decision alone
/// does not start a process or promise that a particular muxer will succeed.
pub fn stream_copy_compatible(
    profile: VideoProfile,
    options: ConversionOptions,
    probe_json: &[u8],
) -> bool {
    if options.trim == TrimRange::default()
        || options.encoder != VideoEncoder::Software
        || options.quality != EncodingQuality::EncoderDefault
        || options.crop.is_some()
        || options.resize != ResizePreset::Original
        || options.rotation != Rotation::None
        || options.audio_bitrate != AudioBitrate::EncoderDefault
        || (profile.is_audio_only() && options.mute_audio)
    {
        return false;
    }
    let (video, audio) = match profile {
        VideoProfile::Mp4 | VideoProfile::Mkv => (Some("h264"), "aac"),
        VideoProfile::Webm => (Some("vp9"), "opus"),
        VideoProfile::Mp4Hevc | VideoProfile::MkvHevc => (Some("hevc"), "aac"),
        VideoProfile::Mp4Av1 => (Some("av1"), "aac"),
        VideoProfile::WebmAv1 => (Some("av1"), "opus"),
        VideoProfile::Mp3Audio => (None, "mp3"),
        VideoProfile::M4aAudio => (None, "aac"),
        VideoProfile::FlacAudio => (None, "flac"),
        VideoProfile::OpusAudio => (None, "opus"),
    };
    let Ok(probe) = serde_json::from_slice::<serde_json::Value>(probe_json) else {
        return false;
    };
    let Some(streams) = probe.get("streams").and_then(|v| v.as_array()) else {
        return false;
    };
    let first = |kind: &str| {
        streams
            .iter()
            .find(|stream| stream.get("codec_type").and_then(|v| v.as_str()) == Some(kind))
    };
    let codec = |stream: &serde_json::Value| {
        stream
            .get("codec_name")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    if let Some(expected) = video {
        if first("video").and_then(codec).as_deref() != Some(expected) {
            return false;
        }
    }
    if !options.mute_audio {
        match first("audio") {
            Some(stream) if codec(stream).as_deref() != Some(audio) => return false,
            None if video.is_none() => return false,
            _ => {}
        }
    }
    true
}

fn video_filters(options: ConversionOptions) -> Option<String> {
    let mut filters = Vec::new();
    if let Some(crop) = options.crop {
        filters.push(format!(
            "crop={}:{}:{}:{}:exact=1",
            crop.width, crop.height, crop.x, crop.y
        ));
    }
    match options.rotation {
        Rotation::None => {}
        Rotation::Clockwise => filters.push("transpose=clock".into()),
        Rotation::HalfTurn => filters.push("hflip,vflip".into()),
        Rotation::CounterClockwise => filters.push("transpose=cclock".into()),
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
    let hardware_codec = options.encoder.codec(profile)?;
    if hardware_codec.is_some() && options.quality != EncodingQuality::EncoderDefault {
        return Err("Hardware encoding requires encoder-default quality".into());
    }
    if profile.is_audio_only() {
        if options.mute_audio {
            return Err("Audio-only output cannot remove audio".into());
        }
        let audio = match profile {
            VideoProfile::Mp3Audio => "libmp3lame",
            VideoProfile::M4aAudio => "aac",
            VideoProfile::FlacAudio => "flac",
            VideoProfile::OpusAudio => "libopus",
            _ => unreachable!(),
        };
        spec = spec.args(["-map", "0:a:0", "-vn", "-c:a", audio]);
    } else {
        let (video, audio) = match profile {
            VideoProfile::Mp4 | VideoProfile::Mkv => ("libx264", "aac"),
            VideoProfile::Webm => ("libvpx-vp9", "libopus"),
            VideoProfile::Mp4Hevc | VideoProfile::MkvHevc => ("libx265", "aac"),
            VideoProfile::Mp4Av1 => ("libaom-av1", "aac"),
            VideoProfile::WebmAv1 => ("libaom-av1", "libopus"),
            _ => unreachable!(),
        };
        let video = hardware_codec.unwrap_or(video);
        if let Some(filters) = video_filters(options) {
            spec = spec.arg("-vf").arg(filters);
        }
        spec = spec.args(["-map", "0:v:0", "-c:v", video]);
        if options.mute_audio {
            spec = spec.arg("-an");
        } else {
            spec = spec.args(["-map", "0:a:0?", "-c:a", audio]);
        }
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
    }
    if !options.mute_audio && profile != VideoProfile::FlacAudio {
        if let Some(bitrate) = options.audio_bitrate.argument() {
            spec = spec.args(["-b:a", bitrate]);
        }
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

fn run_one_with_progress(
    spec: &ProcessSpec,
    token: &CancellationToken,
    index: usize,
    total: usize,
    progress: &mut impl FnMut(BatchProgress),
) -> Result<process::ProcessOutput, process::ProcessError> {
    let mut parser = ProgressParser::default();
    let mut last_report = None;
    let limits = ProcessLimits {
        timeout: std::time::Duration::from_secs(6 * 60 * 60),
        stdout_bytes: 8192,
        stderr_bytes: 64 * 1024,
        line_bytes: 4096,
    };
    process::OwnedProcess::spawn(spec).and_then(|owner| {
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
    })
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
        if token.is_cancelled() {
            return Err(format!("cancelled before file {index} of {total}"));
        }
        run_one_with_progress(spec, token, index, total, &mut progress)
            .map_err(|error| format!("file {index} of {total}: {error}"))?;
    }
    Ok(())
}

/// A validated conversion paired with the information needed for optional trim copy.
#[derive(Debug, Clone)]
pub struct PreparedConversion {
    spec: ProcessSpec,
    input: PathBuf,
    output: PathBuf,
    profile: VideoProfile,
    options: ConversionOptions,
}

/// Validate a single conversion without creating or changing the output.
pub fn prepare_conversion(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    profile: VideoProfile,
    options: ConversionOptions,
) -> Result<PreparedConversion, String> {
    let spec = build_conversion_with_options(ffmpeg, input, output, profile, options)?;
    let input = input
        .canonicalize()
        .map_err(|e| format!("Cannot open input: {e}"))?;
    let output = PathBuf::from(spec.args.last().ok_or("Missing output")?);
    Ok(PreparedConversion {
        spec,
        input,
        output,
        profile,
        options,
    })
}

/// Validate all batch outputs before any process starts.
pub fn prepare_batch(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    output_dir: &Path,
    profile: VideoProfile,
    options: ConversionOptions,
) -> Result<Vec<PreparedConversion>, String> {
    let specs = build_batch_with_options(ffmpeg, inputs, output_dir, profile, options)?;
    inputs
        .iter()
        .zip(specs)
        .map(|(input, spec)| {
            let input = input
                .canonicalize()
                .map_err(|e| format!("Cannot open input: {e}"))?;
            let output = PathBuf::from(spec.args.last().ok_or("Missing output")?);
            Ok(PreparedConversion {
                spec,
                input,
                output,
                profile,
                options,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionMethod {
    StreamCopy,
    Reencode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizedProgress {
    Started {
        index: usize,
        total: usize,
    },
    Method {
        index: usize,
        total: usize,
        method: ConversionMethod,
        fallback: bool,
    },
    MediaTime {
        index: usize,
        total: usize,
        microseconds: u64,
    },
}

fn inspect_streams(ffprobe: &Path, input: &Path, token: &CancellationToken) -> Option<Vec<u8>> {
    let spec = ProcessSpec::new(ffprobe)
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-show_entries",
            "stream=codec_name,codec_type",
            "-of",
            "json",
            "-i",
        ])
        .arg(input);
    process::run_capture(
        &spec,
        ProcessLimits {
            timeout: std::time::Duration::from_secs(15),
            stdout_bytes: 64 * 1024,
            stderr_bytes: 8192,
            line_bytes: 8192,
        },
        token,
    )
    .ok()
    .map(|result| result.stdout)
}

fn copy_spec(job: &PreparedConversion, temp_output: &Path) -> ProcessSpec {
    let trim = job.options.trim;
    let mut spec = ProcessSpec::new(&job.spec.program).args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-progress",
        "pipe:1",
        "-nostats",
        "-n",
        "-protocol_whitelist",
        "file,pipe",
    ]);
    if trim.start_ms > 0 {
        spec = spec.arg("-ss").arg(seconds_arg(trim.start_ms));
    }
    spec = spec.arg("-i").arg(&job.input);
    if let Some(end_ms) = trim.end_ms {
        spec = spec.arg("-t").arg(seconds_arg(end_ms - trim.start_ms));
    }
    if job.profile.is_audio_only() {
        spec = spec.args(["-map", "0:a:0", "-vn"]);
    } else {
        spec = spec.args(["-map", "0:v:0"]);
        if job.options.mute_audio {
            spec = spec.arg("-an");
        } else {
            spec = spec.args(["-map", "0:a:0?"]);
        }
    }
    spec = spec.args(["-c", "copy"]);
    if job.profile == VideoProfile::Mp4Hevc {
        spec = spec.args(["-tag:v", "hvc1"]);
    }
    spec.arg(temp_output)
}

fn reserve_copy_directory(output: &Path) -> Result<PathBuf, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = output.parent().ok_or("Output has no parent")?;
    for _ in 0..100 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = parent.join(format!(".yttraydl-trim-{}-{n}", std::process::id()));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot reserve copy workspace: {e}")),
        }
    }
    Err("Cannot reserve a unique copy workspace".into())
}

fn cleanup_copy_directory(dir: &Path, temp_output: &Path) -> Result<(), String> {
    match std::fs::remove_file(temp_output) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "Cannot remove temporary copy {}: {e}",
                temp_output.display()
            ))
        }
    }
    std::fs::remove_dir(dir)
        .map_err(|e| format!("Cannot remove temporary directory {}: {e}", dir.display()))
}

/// Execute validated jobs, conservatively attempting stream-copy for compatible
/// trims. Copy output is isolated, then published with a non-overwriting hard
/// link. Only ffmpeg's nonzero copy exit triggers re-encoding. Cancellation,
/// I/O, timeout and publication failures never trigger a fallback.
pub fn run_prepared_batch(
    jobs: &[PreparedConversion],
    ffprobe: Option<&Path>,
    token: &CancellationToken,
    mut progress: impl FnMut(OptimizedProgress),
) -> Result<Vec<ConversionMethod>, String> {
    if jobs.is_empty() {
        return Err("No conversions to run".into());
    }
    let total = jobs.len();
    let mut methods = Vec::with_capacity(total);
    for (position, job) in jobs.iter().enumerate() {
        let index = position + 1;
        let fail = |error: String| format!("file {index} of {total}: {error}");
        if token.is_cancelled() {
            return Err(fail("cancelled".into()));
        }
        progress(OptimizedProgress::Started { index, total });
        let compatible = if job.options.trim != TrimRange::default() {
            ffprobe
                .and_then(|probe| inspect_streams(probe, &job.input, token))
                .is_some_and(|data| stream_copy_compatible(job.profile, job.options, &data))
        } else {
            false
        };
        if token.is_cancelled() {
            return Err(fail("cancelled".into()));
        }
        let mut fallback = false;
        if compatible {
            progress(OptimizedProgress::Method {
                index,
                total,
                method: ConversionMethod::StreamCopy,
                fallback: false,
            });
            if token.is_cancelled() {
                return Err(fail("cancelled".into()));
            }
            let dir = reserve_copy_directory(&job.output).map_err(fail)?;
            let temp = dir.join(format!("copy.{}", job.profile.extension()));
            let spec = copy_spec(job, &temp);
            let mut relay = |event| {
                if let BatchProgress::EncodedTime {
                    index,
                    total,
                    microseconds,
                } = event
                {
                    progress(OptimizedProgress::MediaTime {
                        index,
                        total,
                        microseconds,
                    });
                }
            };
            let run = run_one_with_progress(&spec, token, index, total, &mut relay);
            match run {
                Ok(_) if !token.is_cancelled() => {
                    let valid = std::fs::symlink_metadata(&temp)
                        .is_ok_and(|meta| meta.file_type().is_file() && meta.len() > 0);
                    let publication = if valid {
                        std::fs::hard_link(&temp, &job.output).map_err(|e| {
                            format!("Cannot publish copy without overwriting output: {e}")
                        })
                    } else {
                        Err("Copy produced no regular, nonempty output".into())
                    };
                    let cleanup = cleanup_copy_directory(&dir, &temp);
                    if let Err(error) = publication {
                        cleanup.map_err(fail)?;
                        return Err(fail(error));
                    }
                    cleanup.map_err(|e| fail(format!("Output was created, but {e}")))?;
                    methods.push(ConversionMethod::StreamCopy);
                    continue;
                }
                Ok(_) => {
                    cleanup_copy_directory(&dir, &temp).map_err(fail)?;
                    return Err(fail("cancelled".into()));
                }
                Err(error) => {
                    cleanup_copy_directory(&dir, &temp).map_err(fail)?;
                    if matches!(error, process::ProcessError::Exit { .. }) && !token.is_cancelled()
                    {
                        fallback = true;
                    } else {
                        return Err(fail(error.to_string()));
                    }
                }
            }
        }
        progress(OptimizedProgress::Method {
            index,
            total,
            method: ConversionMethod::Reencode,
            fallback,
        });
        if token.is_cancelled() {
            return Err(fail("cancelled".into()));
        }
        let mut relay = |event| {
            if let BatchProgress::EncodedTime {
                index,
                total,
                microseconds,
            } = event
            {
                progress(OptimizedProgress::MediaTime {
                    index,
                    total,
                    microseconds,
                });
            }
        };
        run_one_with_progress(&job.spec, token, index, total, &mut relay)
            .map_err(|e| fail(e.to_string()))?;
        methods.push(ConversionMethod::Reencode);
    }
    Ok(methods)
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
    fn stream_copy_preflight_preserves_profile_and_user_intent() {
        let trim = TrimRange::parse("1", "2").unwrap();
        let options = ConversionOptions {
            trim,
            ..Default::default()
        };
        let compatible = br#"{"streams":[{"codec_type":"video","codec_name":"h264"},{"codec_type":"audio","codec_name":"aac"}]}"#;
        assert!(stream_copy_compatible(
            VideoProfile::Mp4,
            options,
            compatible
        ));
        assert!(!stream_copy_compatible(
            VideoProfile::Webm,
            options,
            compatible
        ));
        assert!(!stream_copy_compatible(
            VideoProfile::Mp4,
            ConversionOptions::default(),
            compatible
        ));
        assert!(!stream_copy_compatible(
            VideoProfile::Mp4,
            ConversionOptions {
                encoder: VideoEncoder::Nvenc,
                ..options
            },
            compatible
        ));
        assert!(!stream_copy_compatible(
            VideoProfile::Mp4,
            ConversionOptions {
                quality: EncodingQuality::High,
                ..options
            },
            compatible
        ));
        assert!(!stream_copy_compatible(
            VideoProfile::Mp4,
            ConversionOptions {
                resize: ResizePreset::P720,
                ..options
            },
            compatible
        ));
        assert!(!stream_copy_compatible(
            VideoProfile::Mp4,
            ConversionOptions {
                rotation: Rotation::HalfTurn,
                ..options
            },
            compatible
        ));
        assert!(!stream_copy_compatible(
            VideoProfile::Mp4,
            ConversionOptions {
                audio_bitrate: AudioBitrate::K128,
                ..options
            },
            compatible
        ));
        assert!(!stream_copy_compatible(
            VideoProfile::Mp4,
            options,
            b"not json"
        ));
        assert!(!stream_copy_compatible(
            VideoProfile::Mp4,
            options,
            br#"{"streams":[]}"#
        ));
        let wrong_audio = br#"{"streams":[{"codec_type":"video","codec_name":"h264"},{"codec_type":"audio","codec_name":"opus"}]}"#;
        assert!(!stream_copy_compatible(
            VideoProfile::Mp4,
            options,
            wrong_audio
        ));
        assert!(stream_copy_compatible(
            VideoProfile::Mp4,
            ConversionOptions {
                mute_audio: true,
                ..options
            },
            wrong_audio
        ));
        let audio = br#"{"streams":[{"codec_type":"audio","codec_name":"mp3"}]}"#;
        assert!(stream_copy_compatible(
            VideoProfile::Mp3Audio,
            options,
            audio
        ));
        assert!(!stream_copy_compatible(
            VideoProfile::M4aAudio,
            options,
            audio
        ));
    }

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
            if profile.is_audio_only() {
                let audio = match profile {
                    VideoProfile::Mp3Audio => "libmp3lame",
                    VideoProfile::M4aAudio => "aac",
                    VideoProfile::FlacAudio => "flac",
                    VideoProfile::OpusAudio => "libopus",
                    _ => unreachable!(),
                };
                assert!(spec
                    .args
                    .windows(2)
                    .any(|p| p[0] == "-c:a" && p[1] == audio));
                assert!(spec.args.iter().any(|p| p == "-vn"));
                assert!(!spec.args.iter().any(|p| p == "-crf" || p == "-vf"));
                continue;
            }
            let video = match profile {
                VideoProfile::Mp4 | VideoProfile::Mkv => "libx264",
                VideoProfile::Webm => "libvpx-vp9",
                VideoProfile::Mp4Hevc | VideoProfile::MkvHevc => "libx265",
                VideoProfile::Mp4Av1 | VideoProfile::WebmAv1 => "libaom-av1",
                _ => unreachable!(),
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
    fn duration_and_estimates_handle_trim_unknown_and_overrun() {
        use std::time::Duration;
        let trim = TrimRange::parse("1", "4").unwrap();
        assert_eq!(trimmed_duration_us(10_000_000, trim), Some(3_000_000));
        assert_eq!(trimmed_duration_us(2_000_000, trim), Some(1_000_000));
        assert_eq!(trimmed_duration_us(500_000, trim), None);
        assert_eq!(trimmed_duration_us(0, TrimRange::default()), None);
        assert_eq!(estimate_progress(1, 0, Duration::ZERO), None);
        assert_eq!(
            estimate_progress(500_000, 2_000_000, Duration::from_secs(4)),
            Some((25, Some(Duration::from_secs(12))))
        );
        assert_eq!(
            estimate_progress(500_000, 2_000_000, Duration::ZERO),
            Some((25, None))
        );
        assert_eq!(
            estimate_progress(3_000_000, 2_000_000, Duration::from_secs(4)),
            Some((99, None))
        );
    }

    #[test]
    fn hardware_encoder_requires_supported_profile_and_default_quality() {
        let dir = std::env::temp_dir().join(format!("yttraydl-hardware-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input.mp4");
        std::fs::write(&input, b"fixture").unwrap();
        for (encoder, h264, hevc) in [
            (VideoEncoder::Nvenc, "h264_nvenc", "hevc_nvenc"),
            (VideoEncoder::Qsv, "h264_qsv", "hevc_qsv"),
            (VideoEncoder::Amf, "h264_amf", "hevc_amf"),
        ] {
            for (profile, codec) in [(VideoProfile::Mp4, h264), (VideoProfile::MkvHevc, hevc)] {
                let options = ConversionOptions {
                    encoder,
                    ..Default::default()
                };
                let spec = build_conversion_with_options(
                    Path::new("ffmpeg"),
                    &input,
                    &dir.join(format!("out.{}", profile.extension())),
                    profile,
                    options,
                )
                .unwrap();
                assert!(spec
                    .args
                    .windows(2)
                    .any(|p| p[0] == "-c:v" && p[1] == codec));
                assert!(!spec.args.iter().any(|p| p == "-crf"));
                assert!(build_conversion_with_options(
                    Path::new("ffmpeg"),
                    &input,
                    &dir.join(format!("out.{}", profile.extension())),
                    profile,
                    ConversionOptions {
                        quality: EncodingQuality::High,
                        ..options
                    }
                )
                .is_err());
            }
            for profile in [
                VideoProfile::Webm,
                VideoProfile::Mp4Av1,
                VideoProfile::Mp3Audio,
            ] {
                assert!(build_conversion_with_options(
                    Path::new("ffmpeg"),
                    &input,
                    &dir.join(format!("out.{}", profile.extension())),
                    profile,
                    ConversionOptions {
                        encoder,
                        ..Default::default()
                    }
                )
                .is_err());
            }
        }
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
    fn audio_bitrate_respects_mute_and_lossless() {
        let dir =
            std::env::temp_dir().join(format!("yttraydl-audio-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input.mkv");
        std::fs::write(&input, b"fixture").unwrap();
        for (profile, muted, expected) in [
            (VideoProfile::Mp3Audio, false, true),
            (VideoProfile::Mp4, false, true),
            (VideoProfile::Mp4, true, false),
            (VideoProfile::FlacAudio, false, false),
        ] {
            let options = ConversionOptions {
                mute_audio: muted,
                audio_bitrate: AudioBitrate::K192,
                ..Default::default()
            };
            let spec = build_conversion_with_options(
                Path::new("ffmpeg"),
                &input,
                &dir.join(format!("output.{}", profile.extension())),
                profile,
                options,
            )
            .unwrap();
            assert_eq!(
                spec.args
                    .windows(2)
                    .any(|p| p[0] == "-b:a" && p[1] == "192k"),
                expected
            );
        }
        assert!(build_conversion_with_options(
            Path::new("ffmpeg"),
            &input,
            &dir.join("x.mp3"),
            VideoProfile::Mp3Audio,
            ConversionOptions {
                mute_audio: true,
                ..Default::default()
            }
        )
        .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rotation_runs_between_crop_and_resize() {
        let crop = CropRect::parse("640", "360", "10", "20").unwrap();
        let options = ConversionOptions {
            crop,
            rotation: Rotation::Clockwise,
            resize: ResizePreset::P720,
            ..Default::default()
        };
        assert!(video_filters(options)
            .unwrap()
            .starts_with("crop=640:360:10:20:exact=1,transpose=clock,scale="));
        for (rotation, expected) in [
            (Rotation::HalfTurn, "hflip,vflip"),
            (Rotation::CounterClockwise, "transpose=cclock"),
        ] {
            assert_eq!(
                video_filters(ConversionOptions {
                    rotation,
                    ..Default::default()
                })
                .as_deref(),
                Some(expected)
            );
        }
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
