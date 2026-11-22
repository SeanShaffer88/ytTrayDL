//! Verified, transactional provisioning of the app-owned Windows media tools.
//!
//! Downloads resolve a concrete official release once, verify its API digest,
//! then publish one manifest only after every selected tool has passed checks.
//! Importing this module performs no I/O. Old generations are retained so a
//! running download never loses its executable; uninstall preserves user data.

use crate::process::{self, CancellationToken, ProcessLimits, ProcessSpec};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const METADATA_CAP: usize = 2 * 1024 * 1024;
const YTDLP_CAP: u64 = 128 * 1024 * 1024;
const ARCHIVE_CAP: u64 = 256 * 1024 * 1024;
const EXPANDED_CAP: u64 = 768 * 1024 * 1024;
const MANIFEST_CAP: u64 = 64 * 1024;

pub type Error = String;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProvisionSelection {
    #[serde(rename = "all")]
    All,
    #[serde(rename = "yt-dlp")]
    YtDlp,
    #[serde(rename = "ffmpeg")]
    FFmpeg,
}
impl ProvisionSelection {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "yt-dlp" => Some(Self::YtDlp),
            "ffmpeg" => Some(Self::FFmpeg),
            _ => None,
        }
    }
    fn families(self) -> &'static [Family] {
        match self {
            Self::All => &[Family::YtDlp, Family::FFmpeg],
            Self::YtDlp => &[Family::YtDlp],
            Self::FFmpeg => &[Family::FFmpeg],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub sha256: String,
    pub source_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema_version: u32,
    pub status: String,
    pub selection: ProvisionSelection,
    pub tools: Vec<ToolInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
impl Report {
    pub fn failure(selection: ProvisionSelection, error: &str) -> Self {
        Self {
            schema_version: 1,
            status: "error".into(),
            selection,
            tools: Vec::new(),
            error: Some(printable(error)),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    tools: Vec<ToolInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    YtDlp,
    FFmpeg,
}
impl Family {
    fn repo(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp/yt-dlp",
            Self::FFmpeg => "GyanD/codexffmpeg",
        }
    }
    fn cap(self) -> u64 {
        match self {
            Self::YtDlp => YTDLP_CAP,
            Self::FFmpeg => ARCHIVE_CAP,
        }
    }
    fn owns(self, name: &str) -> bool {
        match self {
            Self::YtDlp => name == "yt-dlp",
            Self::FFmpeg => matches!(name, "ffmpeg" | "ffprobe"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Asset {
    version: String,
    name: String,
    url: String,
    size: u64,
    sha256: String,
    metadata_url: String,
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
fn resolve_asset(bytes: &[u8], family: Family) -> Result<Asset, Error> {
    if bytes.len() > METADATA_CAP {
        return Err("Release metadata exceeds its limit".into());
    }
    let release: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "Invalid release metadata")?;
    if release["draft"].as_bool() != Some(false) || release["prerelease"].as_bool() != Some(false) {
        return Err("A stable, published release is required".into());
    }
    let version = release["tag_name"]
        .as_str()
        .filter(|tag| valid_component(tag))
        .ok_or("Invalid release version")?;
    let name = match family {
        Family::YtDlp => "yt-dlp.exe".into(),
        Family::FFmpeg => format!("ffmpeg-{version}-essentials_build.zip"),
    };
    let assets = release["assets"]
        .as_array()
        .ok_or("Missing release assets")?;
    let matches: Vec<_> = assets
        .iter()
        .filter(|asset| asset["name"].as_str() == Some(&name))
        .collect();
    if matches.len() != 1 {
        return Err(format!("Expected one {name} release asset"));
    }
    let asset = matches[0];
    let size = asset["size"]
        .as_u64()
        .filter(|size| *size > 0 && *size <= family.cap())
        .ok_or("Invalid release asset size")?;
    let digest = asset["digest"]
        .as_str()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|digest| valid_digest(digest))
        .ok_or("Release asset has no valid SHA256 digest")?;
    let expected = format!(
        "https://github.com/{}/releases/download/{version}/{name}",
        family.repo()
    );
    if asset["browser_download_url"].as_str() != Some(&expected) {
        return Err("Release asset URL is not the expected official HTTPS URL".into());
    }
    Ok(Asset {
        version: version.into(),
        name,
        url: expected,
        size,
        sha256: digest.to_ascii_lowercase(),
        metadata_url: format!(
            "https://api.github.com/repos/{}/releases/latest",
            family.repo()
        ),
    })
}

trait Host {
    fn metadata(&self, family: Family, token: &CancellationToken) -> Result<Vec<u8>, Error>;
    fn download(
        &self,
        asset: &Asset,
        target: &Path,
        token: &CancellationToken,
    ) -> Result<(), Error>;
    fn version(
        &self,
        name: &str,
        executable: &Path,
        token: &CancellationToken,
    ) -> Result<String, Error>;
}
struct OfficialHost;

fn curl() -> Result<PathBuf, Error> {
    let executable =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32/curl.exe");
    if !executable.is_file() {
        return Err("Windows System32 curl.exe is unavailable".into());
    }
    Ok(executable)
}
fn curl_spec(url: &str, max_size: u64) -> Result<ProcessSpec, Error> {
    // curl's documented -q must be the first argument to disable .curlrc.
    // https://curl.se/docs/manpage.html#-q ; protocol limits also cover redirects.
    Ok(ProcessSpec::new(curl()?)
        .args([
            "-q",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--location",
            "--max-redirs",
            "5",
            "--connect-timeout",
            "15",
            "--max-time",
            "600",
            "--fail",
            "--silent",
            "--show-error",
            "--user-agent",
            "ytTrayDL-tool-provisioner",
        ])
        .args(["--max-filesize", &max_size.to_string()])
        .arg("--url")
        .arg(url))
}
fn run(
    spec: &ProcessSpec,
    token: &CancellationToken,
    timeout: u64,
    cap: usize,
) -> Result<Vec<u8>, Error> {
    process::run_capture(
        spec,
        ProcessLimits {
            timeout: Duration::from_secs(timeout),
            stdout_bytes: cap,
            stderr_bytes: 16 * 1024,
            line_bytes: cap,
        },
        token,
    )
    .map(|output| output.stdout)
    .map_err(|error| printable(&error.to_string()))
}
impl Host for OfficialHost {
    fn metadata(&self, family: Family, token: &CancellationToken) -> Result<Vec<u8>, Error> {
        let url = format!(
            "https://api.github.com/repos/{}/releases/latest",
            family.repo()
        );
        run(
            &curl_spec(&url, METADATA_CAP as u64)?,
            token,
            90,
            METADATA_CAP,
        )
    }
    fn download(
        &self,
        asset: &Asset,
        target: &Path,
        token: &CancellationToken,
    ) -> Result<(), Error> {
        run(
            &curl_spec(&asset.url, asset.size)?
                .arg("--output")
                .arg(target),
            token,
            620,
            16 * 1024,
        )
        .map(|_| ())
    }
    fn version(
        &self,
        name: &str,
        executable: &Path,
        token: &CancellationToken,
    ) -> Result<String, Error> {
        let args: &[&str] = if name == "yt-dlp" {
            &["--ignore-config", "--no-cache-dir", "--version"]
        } else {
            &["-version"]
        };
        let output = run(
            &ProcessSpec::new(executable).args(args),
            token,
            30,
            64 * 1024,
        )?;
        String::from_utf8(output).map_err(|_| "Tool version was not UTF-8".into())
    }
}

fn printable(message: &str) -> String {
    message
        .chars()
        .filter(|character| !character.is_control())
        .take(1500)
        .collect()
}
fn check_cancel(token: &CancellationToken) -> Result<(), Error> {
    if token.is_cancelled() {
        Err("Tool provisioning cancelled".into())
    } else {
        Ok(())
    }
}
fn hash_file(path: &Path, cap: u64, token: &CancellationToken) -> Result<(u64, String), Error> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        check_cancel(token)?;
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > cap {
            return Err("Downloaded file exceeds its limit".into());
        }
        hash.update(&buffer[..count]);
    }
    Ok((
        size,
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    ))
}

fn verify_windows_x64(path: &Path) -> Result<(), Error> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut dos = [0; 64];
    file.read_exact(&mut dos)
        .map_err(|_| "Tool has a truncated PE header")?;
    if &dos[..2] != b"MZ" {
        return Err("Tool is not a Windows executable".into());
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    if !(64..=1024 * 1024).contains(&offset) {
        return Err("Invalid tool PE header offset".into());
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    let mut pe = [0; 26];
    file.read_exact(&mut pe)
        .map_err(|_| "Tool has a truncated PE header")?;
    if &pe[..4] != b"PE\0\0"
        || u16::from_le_bytes([pe[4], pe[5]]) != 0x8664
        || u16::from_le_bytes([pe[24], pe[25]]) != 0x20b
    {
        return Err("Tool must be a Windows x64 PE32+ executable".into());
    }
    Ok(())
}

fn owned_existing_file(root: &Path, relative: &Path) -> bool {
    let Ok(canonical_root) = fs::canonicalize(root) else {
        return false;
    };
    let path = root.join(relative);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return false;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    fs::canonicalize(path).is_ok_and(|canonical| canonical == canonical_root.join(relative))
}

fn safe_archive_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && !name.starts_with('/')
        && !name.contains(['\\', ':'])
        && name.trim_end_matches('/').split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with([' ', '.'])
                && !part.chars().any(char::is_control)
        })
}

fn extract_ffmpeg(
    archive: &Path,
    destination: &Path,
    token: &CancellationToken,
) -> Result<Vec<(String, PathBuf)>, Error> {
    let file = File::open(archive).map_err(|error| error.to_string())?;
    // This sanctioned iterable API avoids an index allocation before the entry
    // cap is checked. Its documented contract requires an exact crate version.
    // https://docs.rs/zip/9.8.1/zip/unstable/read/struct.ZipIterable.html
    let mut zip = zip::unstable::read::ZipIterable::try_new(file, zip::read::Config::default())
        .map_err(|_| "Invalid ffmpeg ZIP archive")?
        .enforce_header_consistency(true);
    if zip.len() > 2048 {
        return Err("Too many archive entries".into());
    }
    let mut seen = HashSet::new();
    let mut selected = Vec::new();
    let mut total = 0u64;
    for entry in zip.files().map_err(|_| "Invalid archive directory")? {
        check_cancel(token)?;
        let entry = entry.map_err(|_| "Invalid archive entry")?;
        let name = entry
            .name()
            .map_err(|_| "Invalid archive filename")?
            .into_owned();
        let mode = entry.unix_mode().unwrap_or(0) & 0o170000;
        if !safe_archive_name(&name)
            || !seen.insert(name.to_ascii_lowercase())
            || entry.is_symlink()
            || !matches!(mode, 0 | 0o100000 | 0o040000)
            || entry.external_attributes() & 0x400 != 0
        {
            return Err("Archive contains an unsafe path, duplicate, or nonregular entry".into());
        }
        total = total
            .checked_add(entry.size())
            .filter(|size| *size <= EXPANDED_CAP)
            .ok_or("Archive expansion exceeds its limit")?;
        if entry.is_dir() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let leaf = lower.rsplit('/').next().unwrap_or("");
        let target = if lower.ends_with("/bin/ffmpeg.exe") {
            Some("ffmpeg.exe".to_owned())
        } else if lower.ends_with("/bin/ffprobe.exe") {
            Some("ffprobe.exe".to_owned())
        } else if matches!(
            leaf,
            "license" | "license.txt" | "copying" | "copying.txt" | "readme" | "readme.txt"
        ) {
            Some(format!("notice-{}-{leaf}", selected.len()))
        } else {
            None
        };
        if let Some(target) = target {
            let limit = if target.ends_with(".exe") {
                256 * 1024 * 1024
            } else {
                1024 * 1024
            };
            if entry.size() == 0 || entry.size() > limit {
                return Err("Invalid selected archive entry size".into());
            }
            let size = entry.size();
            selected.push((entry, target, size));
        }
    }
    for required in ["ffmpeg.exe", "ffprobe.exe"] {
        if selected
            .iter()
            .filter(|(_, target, _)| target == required)
            .count()
            != 1
        {
            return Err(format!("Expected one regular {required}"));
        }
    }
    if !selected
        .iter()
        .any(|(_, target, _)| target.contains("license") || target.contains("copying"))
        || !selected
            .iter()
            .any(|(_, target, _)| target.contains("readme"))
    {
        return Err("Archive is missing license/source notices".into());
    }
    let mut tools = Vec::new();
    drop(zip);
    let mut body = File::open(archive).map_err(|error| error.to_string())?;
    for (entry, name, expected) in selected {
        check_cancel(token)?;
        let target = destination.join(&name);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| error.to_string())?;
        let mut entry = entry
            .with_reader(&mut body, zip::ZipReadOptions::default())
            .map_err(|_| "Invalid selected archive entry")?;
        let mut size = 0;
        let mut buffer = [0; 64 * 1024];
        loop {
            check_cancel(token)?;
            let count = entry
                .read(&mut buffer)
                .map_err(|_| "Archive entry failed integrity check")?;
            if count == 0 {
                break;
            }
            size += count as u64;
            if size > expected {
                return Err("Archive entry exceeded its declared size".into());
            }
            output
                .write_all(&buffer[..count])
                .map_err(|error| error.to_string())?;
        }
        if size != expected {
            return Err("Truncated archive entry".into());
        }
        output.sync_all().map_err(|error| error.to_string())?;
        if matches!(name.as_str(), "ffmpeg.exe" | "ffprobe.exe") {
            tools.push((name.trim_end_matches(".exe").to_owned(), target));
        }
    }
    Ok(tools)
}

fn load_manifest(root: &Path) -> Result<Manifest, Error> {
    let path = root.join("current-tools.json");
    if !path.exists() {
        return Ok(Manifest {
            schema_version: 1,
            tools: Vec::new(),
        });
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| error.to_string())?
        .take(MANIFEST_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MANIFEST_CAP {
        return Err("Managed tool manifest exceeds its limit".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid managed tool manifest")?;
    let mut names = HashSet::new();
    if manifest.schema_version != 1 || manifest.tools.len() > 3 {
        return Err("Unsupported managed tool manifest".into());
    }
    for tool in &manifest.tools {
        let components: Vec<_> = tool.path.components().collect();
        if !matches!(tool.name.as_str(), "yt-dlp" | "ffmpeg" | "ffprobe")
            || !names.insert(&tool.name)
            || !valid_digest(&tool.sha256)
            || components.len() != 3
            || components
                .iter()
                .any(|component| !matches!(component, Component::Normal(_)))
            || components[0].as_os_str() != "generations"
            || !components[1]
                .as_os_str()
                .to_string_lossy()
                .starts_with("generation-")
            || !valid_component(&components[1].as_os_str().to_string_lossy())
            || components[2].as_os_str() != format!("{}.exe", tool.name).as_str()
        {
            return Err("Unsafe managed tool manifest path".into());
        }
        if !owned_existing_file(root, &tool.path) {
            return Err("Managed tool is missing or escapes its owned generation".into());
        }
    }
    let ffmpeg = manifest.tools.iter().find(|tool| tool.name == "ffmpeg");
    let ffprobe = manifest.tools.iter().find(|tool| tool.name == "ffprobe");
    if ffmpeg.is_some() != ffprobe.is_some()
        || ffmpeg
            .zip(ffprobe)
            .is_some_and(|(ffmpeg, ffprobe)| ffmpeg.path.parent() != ffprobe.path.parent())
    {
        return Err("Managed ffmpeg and ffprobe must be one complete generation".into());
    }
    Ok(manifest)
}

fn managed_path_at(root: &Path, name: &str) -> PathBuf {
    if let Ok(manifest) = load_manifest(root) {
        if let Some(tool) = manifest.tools.iter().find(|tool| tool.name == name) {
            let path = root.join(&tool.path);
            if path.is_file() {
                return path;
            }
        }
    }
    root.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    })
}
pub fn managed_ytdlp_path() -> PathBuf {
    managed_path_at(&crate::settings::Settings::managed_bin_dir(), "yt-dlp")
}
pub fn managed_ffmpeg_path() -> PathBuf {
    managed_path_at(&crate::settings::Settings::managed_bin_dir(), "ffmpeg")
}

struct Stage {
    path: PathBuf,
    committed: bool,
}
impl Stage {
    fn create(path: PathBuf) -> Result<Self, Error> {
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self {
            path,
            committed: false,
        })
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub fn provision(
    selection: ProvisionSelection,
    token: &CancellationToken,
    progress: impl FnMut(&str),
) -> Result<Report, Error> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err("Media tool provisioning supports Windows x64 only".into());
    }
    provision_at(
        &crate::settings::Settings::managed_bin_dir(),
        selection,
        token,
        progress,
        &OfficialHost,
    )
}

fn provision_at(
    root: &Path,
    selection: ProvisionSelection,
    token: &CancellationToken,
    mut progress: impl FnMut(&str),
    host: &impl Host,
) -> Result<Report, Error> {
    check_cancel(token)?;
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    fs::create_dir_all(root.join("generations")).map_err(|error| error.to_string())?;
    if fs::canonicalize(root.join("generations")).map_err(|error| error.to_string())?
        != canonical_root.join("generations")
    {
        return Err("Managed generations directory escapes its owned root".into());
    }
    // Standard-library native file locking is available at the project's Rust 1.89 MSRV.
    // https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("provision.lock"))
        .map_err(|error| error.to_string())?;
    lock.try_lock()
        .map_err(|_| "Another media tool operation is already running")?;
    let mut manifest = load_manifest(root)?;
    let generation = format!(
        "generation-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    );
    let stage_path = root.join("generations").join(&generation);
    // Cleanup is armed only after exclusive creation established ownership.
    let mut stage = Stage::create(stage_path)?;
    let mut installed = Vec::new();
    for family in selection.families() {
        check_cancel(token)?;
        progress("Checking Latest Official Release");
        let asset = resolve_asset(&host.metadata(*family, token)?, *family)?;
        let download = stage.path.join(&asset.name);
        progress("Downloading Media Tools");
        host.download(&asset, &download, token)?;
        progress("Verifying Download");
        let (size, hash) = hash_file(&download, family.cap(), token)?;
        if size != asset.size || hash != asset.sha256 {
            return Err(
                "Downloaded media tool size or SHA256 did not match the official release".into(),
            );
        }
        let tools = if *family == Family::YtDlp {
            vec![("yt-dlp".into(), download.clone())]
        } else {
            progress("Checking And Extracting Archive");
            extract_ffmpeg(&download, &stage.path, token)?
        };
        for (name, executable) in tools {
            check_cancel(token)?;
            progress("Checking Tool Versions");
            verify_windows_x64(&executable)?;
            let version = host.version(&name, &executable, token)?;
            let expected = if name == "yt-dlp" {
                asset.version.clone()
            } else {
                format!("{name} version {}", asset.version)
            };
            let line = version.lines().next().unwrap_or("");
            if (name == "yt-dlp" && line != expected)
                || (name != "yt-dlp"
                    && !line
                        .strip_prefix(&expected)
                        .is_some_and(|rest| rest.starts_with('-') || rest.starts_with(' ')))
            {
                return Err(format!("Unexpected {name} version"));
            }
            let (_, sha256) = hash_file(&executable, EXPANDED_CAP, token)?;
            installed.push(ToolInfo {
                name,
                version: asset.version.clone(),
                path: PathBuf::from("generations")
                    .join(&generation)
                    .join(executable.file_name().ok_or("Invalid staged executable")?),
                sha256,
                source_url: asset.url.clone(),
            });
        }
        manifest.tools.retain(|tool| !family.owns(&tool.name));
        let provenance = stage.path.join(if *family == Family::YtDlp {
            "yt-dlp-provenance.json"
        } else {
            "ffmpeg-provenance.json"
        });
        let mut value = serde_json::to_value(&asset).map_err(|error| error.to_string())?;
        if *family == Family::YtDlp {
            value["licenseUrl"] = format!(
                "https://github.com/{}/blob/{}/LICENSE",
                family.repo(),
                asset.version
            )
            .into();
            value["sourceUrl"] = format!(
                "https://github.com/{}/tree/{}",
                family.repo(),
                asset.version
            )
            .into();
        } else {
            let notices: Vec<_> = fs::read_dir(&stage.path)
                .map_err(|error| error.to_string())?
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("notice-"))
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            let readme = notices
                .iter()
                .find(|name| name.ends_with("readme") || name.ends_with("readme.txt"))
                .ok_or("Missing FFmpeg source README")?;
            let text = fs::read_to_string(stage.path.join(readme))
                .map_err(|_| "Invalid FFmpeg source README")?;
            let source = text
                .lines()
                .find_map(|line| line.strip_prefix("Source Code:").map(str::trim))
                .filter(|source| {
                    source
                        .strip_prefix("https://github.com/FFmpeg/FFmpeg/commit/")
                        .is_some_and(|commit| {
                            (7..=64).contains(&commit.len())
                                && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                })
                .ok_or("FFmpeg README has no supported source commit reference")?;
            value["sourceUrl"] = source.into();
            value["retainedNotices"] =
                serde_json::to_value(notices).map_err(|error| error.to_string())?;
            value["providerRepository"] = "https://github.com/GyanD/codexffmpeg".into();
        }
        fs::write(
            provenance,
            serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        if *family == Family::FFmpeg {
            fs::remove_file(download).map_err(|error| error.to_string())?;
        }
    }
    check_cancel(token)?;
    manifest.tools.extend(installed.iter().cloned());
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MANIFEST_CAP {
        return Err("Managed tool manifest exceeds its limit".into());
    }
    let temporary = stage.path.join("current-tools.next.json");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())?;
    drop(file);
    check_cancel(token)?;
    progress("Activating Verified Tools");
    check_cancel(token)?;
    fs::rename(&temporary, root.join("current-tools.json"))
        .map_err(|error| format!("Could not activate verified tools: {error}"))?;
    stage.committed = true;
    for tool in &mut installed {
        tool.path = root.join(&tool.path);
    }
    Ok(Report {
        schema_version: 1,
        status: "success".into(),
        selection,
        tools: installed,
        error: None,
    })
}

/// New report files only; failure never overwrites another operation's report.
fn write_report(file: &mut File, report: &Report) -> Result<(), Error> {
    let bytes = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MANIFEST_CAP {
        return Err("Provision report exceeds its limit".into());
    }
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())
}

/// Headless installer interface. Returns before the caller initializes any UI,
/// clipboard listener, single-instance signal, startup key or tray icon.
pub fn run_cli(args: &[String]) -> Option<i32> {
    if !args.iter().any(|arg| arg == "--provision-tools") {
        return None;
    }
    let parsed = (args.len() == 5 && args[1] == "--provision-tools" && args[3] == "--report")
        .then(|| ProvisionSelection::parse(&args[2]))
        .flatten();
    let Some(selection) = parsed else {
        eprintln!("Expected --provision-tools all|yt-dlp|ffmpeg --report <new-absolute-file>");
        return Some(2);
    };
    let path = Path::new(&args[4]);
    if !path.is_absolute() {
        return Some(2);
    }
    // Reserve the new owned report before network or activation side effects.
    let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(path) else {
        return Some(2);
    };
    let (report, code) = match provision(selection, &CancellationToken::default(), |_| {}) {
        Ok(report) => (report, 0),
        Err(error) => (Report::failure(selection, &error), 2),
    };
    if write_report(&mut file, &report).is_err() {
        return Some(3);
    }
    Some(code)
}

#[cfg(test)]
mod tests;
