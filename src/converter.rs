//! Local video conversion: validate paths and construct a shell-free ffmpeg invocation.
use crate::process::ProcessSpec;
use crate::settings::Settings;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoProfile {
    #[default]
    Mp4,
    Mkv,
    Webm,
}

impl VideoProfile {
    pub const ALL: [Self; 3] = [Self::Mp4, Self::Mkv, Self::Webm];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4 (H.264 + AAC)",
            Self::Mkv => "MKV (H.264 + AAC)",
            Self::Webm => "WebM (VP9 + Opus)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
        }
    }
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
    };
    Ok(ProcessSpec::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-n", "-i"])
        .arg(input)
        .args([
            "-map", "0:v:0", "-map", "0:a:0?", "-c:v", video, "-c:a", audio,
        ])
        .arg(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_arguments_and_existing_files_are_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("yttraydl-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("source & clip.mov");
        let output = dir.join("result.mp4");
        std::fs::write(&input, b"input").unwrap();
        let spec = build_conversion(Path::new("ffmpeg"), &input, &output, VideoProfile::Mp4).unwrap();
        assert!(spec.args.iter().any(|arg| arg == "-n"));
        assert!(spec.args.iter().any(|arg| arg.as_os_str() == input.canonicalize().unwrap().as_os_str()));
        assert!(spec.args.iter().any(|arg| arg.as_os_str() == dir.canonicalize().unwrap().join("result.mp4").as_os_str()));
        assert!(build_conversion(Path::new("ffmpeg"), &input, &input, VideoProfile::Mp4).is_err());
        assert!(build_conversion(Path::new("ffmpeg"), &input, &output, VideoProfile::Webm).is_err());
        std::fs::write(&output, b"do not replace").unwrap();
        assert!(build_conversion(Path::new("ffmpeg"), &input, &output, VideoProfile::Mp4).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"do not replace");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_input_is_rejected() {
        assert!(build_conversion(Path::new("ffmpeg"), Path::new("missing-file"), Path::new("out.mp4"), VideoProfile::Mp4).is_err());
    }
}
