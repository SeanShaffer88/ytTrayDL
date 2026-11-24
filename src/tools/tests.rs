use super::*;
use serde_json::{json, Value};
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TestRoot(PathBuf);
impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yttraydl-tools-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn pe() -> Vec<u8> {
    let mut bytes = vec![0; 96];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
    bytes[64..68].copy_from_slice(b"PE\0\0");
    bytes[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes
}
fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn good_archive() -> Vec<u8> {
    archive(&[
        ("package/bin/ffmpeg.exe", &pe()),
        ("package/bin/ffprobe.exe", &pe()),
        ("package/LICENSE", b"GPL version 3 fixture"),
        (
            "package/README.txt",
            b"Source Code: https://github.com/FFmpeg/FFmpeg/commit/abcdef1234\n",
        ),
    ])
}
fn metadata(family: Family, bytes: &[u8]) -> Value {
    let version = if family == Family::YtDlp {
        "2026.10.26"
    } else {
        "9.1"
    };
    let name = if family == Family::YtDlp {
        "yt-dlp.exe"
    } else {
        "ffmpeg-9.1-essentials_build.zip"
    };
    json!({"tag_name":version,"draft":false,"prerelease":false,"assets":[{"name":name,"size":bytes.len(),"digest":format!("sha256:{}",digest(bytes)),"browser_download_url":format!("https://github.com/{}/releases/download/{version}/{name}",family.repo())}]})
}
struct FakeHost {
    ytdlp: Vec<u8>,
    ffmpeg: Vec<u8>,
    fail: Option<Family>,
    corrupt: bool,
    truncate: bool,
    cancel: Option<Family>,
    bad_version: bool,
    version_calls: AtomicUsize,
    loopback: Option<std::net::SocketAddr>,
}
impl Default for FakeHost {
    fn default() -> Self {
        Self {
            ytdlp: pe(),
            ffmpeg: good_archive(),
            fail: None,
            corrupt: false,
            truncate: false,
            cancel: None,
            bad_version: false,
            version_calls: AtomicUsize::new(0),
            loopback: None,
        }
    }
}
impl Host for FakeHost {
    fn metadata(&self, family: Family, _: &CancellationToken) -> Result<Vec<u8>, Error> {
        Ok(serde_json::to_vec(&metadata(
            family,
            if family == Family::YtDlp {
                &self.ytdlp
            } else {
                &self.ffmpeg
            },
        ))
        .unwrap())
    }
    fn download(
        &self,
        asset: &Asset,
        path: &Path,
        token: &CancellationToken,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<(), Error> {
        let family = if asset.name == "yt-dlp.exe" {
            Family::YtDlp
        } else {
            Family::FFmpeg
        };
        let mut bytes = if family == Family::YtDlp {
            self.ytdlp.clone()
        } else {
            self.ffmpeg.clone()
        };
        if let Some(address) = self.loopback {
            let mut socket = std::net::TcpStream::connect_timeout(&address, Duration::from_secs(2))
                .map_err(|error| error.to_string())?;
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .write_all(b"GET /fixture HTTP/1.0\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut response = Vec::new();
            socket.take(1024 * 1024).read_to_end(&mut response).unwrap();
            let start = response
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap()
                + 4;
            bytes = response[start..].to_vec();
        }
        if self.corrupt {
            bytes[0] ^= 1;
        }
        if self.truncate {
            bytes.pop();
        }
        progress(bytes.len() as u64, Some(asset.size));
        fs::write(path, bytes).unwrap();
        if self.cancel == Some(family) {
            token.cancel();
        }
        if self.fail == Some(family) {
            return Err("injected transport error".into());
        }
        Ok(())
    }
    fn version(&self, name: &str, _: &Path, _: &CancellationToken) -> Result<String, Error> {
        self.version_calls.fetch_add(1, Ordering::SeqCst);
        Ok(if self.bad_version {
            "wrong".into()
        } else if name == "yt-dlp" {
            "2026.10.26".into()
        } else {
            format!("{name} version 9.1-fixture\n")
        })
    }
}
fn install(root: &Path, selection: ProvisionSelection, host: &FakeHost) -> Result<Report, Error> {
    provision_at(root, selection, &CancellationToken::default(), |_| {}, host)
}

#[test]
fn release_metadata_requires_stable_unique_official_bounded_digest_asset() {
    let good = metadata(Family::YtDlp, &pe());
    assert!(resolve_asset(&serde_json::to_vec(&good).unwrap(), Family::YtDlp).is_ok());
    let mut cases = vec![Value::Null];
    for field in ["draft", "prerelease"] {
        let mut value = good.clone();
        value[field] = true.into();
        cases.push(value);
    }
    let mut value = good.clone();
    value["tag_name"] = "../bad".into();
    cases.push(value);
    let mut value = good.clone();
    value["assets"]
        .as_array_mut()
        .unwrap()
        .push(good["assets"][0].clone());
    cases.push(value);
    for (field, invalid) in [
        ("digest", json!("sha256:no")),
        ("digest", json!(null)),
        ("size", json!(YTDLP_CAP + 1)),
        ("size", json!(0)),
        (
            "browser_download_url",
            json!("http://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp.exe"),
        ),
        (
            "browser_download_url",
            json!("https://evil.invalid/yt-dlp.exe"),
        ),
    ] {
        let mut value = good.clone();
        value["assets"][0][field] = invalid;
        cases.push(value);
    }
    for value in cases {
        assert!(
            resolve_asset(&serde_json::to_vec(&value).unwrap(), Family::YtDlp).is_err(),
            "{value}"
        );
    }
    assert!(resolve_asset(&vec![b' '; METADATA_CAP + 1], Family::YtDlp).is_err());
}

#[test]
fn all_tools_commit_once_pair_versions_and_keep_other_family_and_legacy_files() {
    let root = TestRoot::new();
    let host = FakeHost::default();
    fs::write(root.0.join("yt-dlp.exe"), b"legacy").unwrap();
    let report = install(&root.0, ProvisionSelection::All, &host).unwrap();
    assert_eq!(report.tools.len(), 3);
    assert_eq!(report.status, "success");
    let first = load_manifest(&root.0).unwrap();
    assert_eq!(
        first
            .tools
            .iter()
            .find(|tool| tool.name == "ffmpeg")
            .unwrap()
            .path
            .parent(),
        first
            .tools
            .iter()
            .find(|tool| tool.name == "ffprobe")
            .unwrap()
            .path
            .parent()
    );
    let ffmpeg = managed_path_at(&root.0, "ffmpeg");
    install(&root.0, ProvisionSelection::YtDlp, &host).unwrap();
    assert_eq!(managed_path_at(&root.0, "ffmpeg"), ffmpeg);
    assert_eq!(fs::read(root.0.join("yt-dlp.exe")).unwrap(), b"legacy");
    assert!(report.tools.iter().all(|tool| tool.path.is_file()));
}

#[test]
fn errors_cancellation_hash_truncation_and_version_failure_preserve_previous_manifest() {
    let root = TestRoot::new();
    install(&root.0, ProvisionSelection::All, &FakeHost::default()).unwrap();
    let before = fs::read(root.0.join("current-tools.json")).unwrap();
    let generation_count = fs::read_dir(root.0.join("generations")).unwrap().count();
    for host in [
        FakeHost {
            fail: Some(Family::FFmpeg),
            ..Default::default()
        },
        FakeHost {
            corrupt: true,
            ..Default::default()
        },
        FakeHost {
            truncate: true,
            ..Default::default()
        },
        FakeHost {
            cancel: Some(Family::FFmpeg),
            ..Default::default()
        },
        FakeHost {
            bad_version: true,
            ..Default::default()
        },
    ] {
        assert!(install(&root.0, ProvisionSelection::All, &host).is_err());
        assert_eq!(fs::read(root.0.join("current-tools.json")).unwrap(), before);
        assert_eq!(
            fs::read_dir(root.0.join("generations")).unwrap().count(),
            generation_count
        );
    }
    install(&root.0, ProvisionSelection::All, &FakeHost::default()).unwrap();
}

#[test]
fn unsafe_zip_missing_pair_and_wrong_architecture_never_execute_or_activate() {
    let executable = pe();
    for bad_name in [
        "../escape",
        "/absolute",
        "C:/drive",
        "pkg\\escape",
        "pkg/trailing.",
        "pkg/FILE",
    ] {
        let root = TestRoot::new();
        let bytes = archive(&[
            ("package/bin/ffmpeg.exe", &executable),
            ("package/bin/ffprobe.exe", &executable),
            ("pkg/file", b"one"),
            (bad_name, b"two"),
        ]);
        let host = FakeHost {
            ffmpeg: bytes,
            ..Default::default()
        };
        assert!(install(&root.0, ProvisionSelection::FFmpeg, &host).is_err());
        assert_eq!(host.version_calls.load(Ordering::SeqCst), 0);
        assert!(!root.0.join("current-tools.json").exists());
    }
    let root = TestRoot::new();
    let host = FakeHost {
        ffmpeg: archive(&[("package/bin/ffmpeg.exe", &executable)]),
        ..Default::default()
    };
    assert!(install(&root.0, ProvisionSelection::FFmpeg, &host).is_err());
    let mut wrong = pe();
    wrong[68..70].copy_from_slice(&0x14cu16.to_le_bytes());
    let host = FakeHost {
        ytdlp: wrong,
        ..Default::default()
    };
    assert!(install(&root.0, ProvisionSelection::YtDlp, &host).is_err());
    assert_eq!(host.version_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn symlink_zip_and_existing_stage_collision_preserve_foreign_data() {
    let root = TestRoot::new();
    let collision = root.0.join("generation-collision");
    fs::create_dir(&collision).unwrap();
    fs::write(collision.join("sentinel"), b"keep").unwrap();
    assert!(Stage::create(collision.clone()).is_err());
    assert_eq!(fs::read(collision.join("sentinel")).unwrap(), b"keep");
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.add_symlink(
        "package/bin/ffmpeg.exe",
        "outside",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    let host = FakeHost {
        ffmpeg: zip.finish().unwrap().into_inner(),
        ..Default::default()
    };
    assert!(install(&root.0, ProvisionSelection::FFmpeg, &host).is_err());
}

#[test]
fn zip_entry_count_local_header_and_notice_budgets_fail_before_execution() {
    let root = TestRoot::new();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..2049 {
        writer
            .start_file(
                format!("package/entry-{index}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"x").unwrap();
    }
    let mut mismatched = good_archive();
    let position = mismatched
        .windows(b"package/bin/ffmpeg.exe".len())
        .position(|bytes| bytes == b"package/bin/ffmpeg.exe")
        .unwrap();
    mismatched[position] = b'X';
    let mut declared_count = good_archive();
    let end = declared_count
        .windows(4)
        .rposition(|bytes| bytes == b"PK\x05\x06")
        .unwrap();
    declared_count[end + 8..end + 10].copy_from_slice(&u16::MAX.to_le_bytes());
    declared_count[end + 10..end + 12].copy_from_slice(&u16::MAX.to_le_bytes());
    let oversized_notice = archive(&[
        ("package/bin/ffmpeg.exe", &pe()),
        ("package/bin/ffprobe.exe", &pe()),
        ("package/LICENSE", &vec![b'x'; 1024 * 1024 + 1]),
        ("package/README.txt", b"source"),
    ]);
    for bytes in [
        writer.finish().unwrap().into_inner(),
        mismatched,
        declared_count,
        oversized_notice,
    ] {
        let host = FakeHost {
            ffmpeg: bytes,
            ..Default::default()
        };
        assert!(install(&root.0, ProvisionSelection::FFmpeg, &host).is_err());
        assert_eq!(host.version_calls.load(Ordering::SeqCst), 0);
        assert!(!root.0.join("current-tools.json").exists());
    }
}

#[test]
fn native_file_lock_excludes_concurrent_operations_and_releases_after_failure() {
    let root = TestRoot::new();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(root.0.join("provision.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(
        install(&root.0, ProvisionSelection::All, &FakeHost::default())
            .unwrap_err()
            .contains("already running")
    );
    drop(lock);
    install(&root.0, ProvisionSelection::All, &FakeHost::default()).unwrap();
}

#[test]
fn manifests_reject_missing_mixed_or_escaping_tools_and_resolver_retains_legacy() {
    let root = TestRoot::new();
    fs::write(
        root.0.join(if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        }),
        b"legacy",
    )
    .unwrap();
    assert!(managed_path_at(&root.0, "yt-dlp").is_file());
    install(&root.0, ProvisionSelection::All, &FakeHost::default()).unwrap();
    let good = load_manifest(&root.0).unwrap();
    let mut cases = Vec::new();
    let mut lone = good.clone();
    lone.tools.retain(|tool| tool.name != "ffprobe");
    cases.push(lone);
    let mut mixed = good.clone();
    fs::create_dir(root.0.join("generations/generation-other")).unwrap();
    fs::write(
        root.0.join("generations/generation-other/ffprobe.exe"),
        pe(),
    )
    .unwrap();
    mixed
        .tools
        .iter_mut()
        .find(|tool| tool.name == "ffprobe")
        .unwrap()
        .path = PathBuf::from("generations/generation-other/ffprobe.exe");
    cases.push(mixed);
    let mut escape = good.clone();
    escape.tools[0].path = PathBuf::from("generations/../yt-dlp.exe");
    cases.push(escape);
    let mut missing = good.clone();
    missing.tools[0].path = PathBuf::from("generations/generation-missing/yt-dlp.exe");
    cases.push(missing);
    for bad in cases {
        fs::write(
            root.0.join("current-tools.json"),
            serde_json::to_vec(&bad).unwrap(),
        )
        .unwrap();
        assert!(load_manifest(&root.0).is_err());
    }
}

#[test]
fn loopback_truncated_transport_cannot_activate_and_full_transport_can() {
    for truncated in [true, false] {
        let root = TestRoot::new();
        let body = pe();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 512];
            let _ = socket.read(&mut request);
            write!(
                socket,
                "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket
                .write_all(if truncated {
                    &body[..body.len() - 1]
                } else {
                    &body
                })
                .unwrap();
        });
        let host = FakeHost {
            loopback: Some(address),
            ..Default::default()
        };
        assert_eq!(
            install(&root.0, ProvisionSelection::YtDlp, &host).is_ok(),
            !truncated
        );
        server.join().unwrap();
        assert_eq!(root.0.join("current-tools.json").exists(), !truncated);
    }
}

#[test]
fn cli_rejects_bad_contract_existing_report_and_preserves_manual_launch() {
    assert_eq!(run_cli(&["app".into(), "--background".into()]), None);
    assert_eq!(
        run_cli(&["app".into(), "--provision-tools".into()]),
        Some(2)
    );
    let root = TestRoot::new();
    let report = root.0.join("report.json");
    fs::write(&report, b"sentinel").unwrap();
    assert_eq!(
        run_cli(&[
            "app".into(),
            "--provision-tools".into(),
            "all".into(),
            "--report".into(),
            report.to_string_lossy().into_owned()
        ]),
        Some(2)
    );
    assert_eq!(fs::read(report).unwrap(), b"sentinel");
}

#[test]
fn typed_stages_cancel_before_activation_and_complete_only_after_commit() {
    let root = TestRoot::new();
    install(&root.0, ProvisionSelection::All, &FakeHost::default()).unwrap();
    let before = fs::read(root.0.join("current-tools.json")).unwrap();
    for stage in [
        ToolStage::Download,
        ToolStage::Verify,
        ToolStage::Extract,
        ToolStage::CheckVersions,
        ToolStage::Activate,
    ] {
        let token = CancellationToken::default();
        let mut events = Vec::new();
        let result = provision_at_with_progress(
            &root.0,
            ProvisionSelection::All,
            &token,
            99,
            |update| {
                if update.stage == stage {
                    token.cancel();
                }
                events.push(update);
            },
            &FakeHost::default(),
        );
        assert!(result.is_err(), "{stage:?}");
        assert_eq!(fs::read(root.0.join("current-tools.json")).unwrap(), before);
        assert!(events
            .iter()
            .all(|event| event.operation_id == 99 && event.stage != ToolStage::Complete));
    }
    let mut completed = Vec::new();
    provision_at_with_progress(
        &root.0,
        ProvisionSelection::All,
        &CancellationToken::default(),
        100,
        |update| {
            if update.stage == ToolStage::Complete {
                assert_ne!(fs::read(root.0.join("current-tools.json")).unwrap(), before);
                assert_eq!(load_manifest(&root.0).unwrap().tools.len(), 3);
                assert_eq!(Some(update.bytes_done), update.total_bytes);
                completed.push(update.family);
            }
        },
        &FakeHost::default(),
    )
    .unwrap();
    assert_eq!(completed, vec![ToolFamily::YtDlp, ToolFamily::FFmpeg]);
}

#[cfg(windows)]
#[test]
fn actual_owned_curl_cancel_and_oversize_roll_back_then_independent_retry_succeeds() {
    use std::net::{TcpListener, TcpStream};
    struct RawHost {
        address: std::net::SocketAddr,
        base: FakeHost,
        overflow: bool,
    }
    impl Host for RawHost {
        fn metadata(&self, family: Family, token: &CancellationToken) -> Result<Vec<u8>, Error> {
            self.base.metadata(family, token)
        }
        fn version(
            &self,
            name: &str,
            path: &Path,
            token: &CancellationToken,
        ) -> Result<String, Error> {
            self.base.version(name, path, token)
        }
        fn download(
            &self,
            asset: &Asset,
            path: &Path,
            token: &CancellationToken,
            progress: &mut dyn FnMut(u64, Option<u64>),
        ) -> Result<(), Error> {
            let spec = ProcessSpec::new(curl()?).args([
                "-q",
                "--silent",
                "--show-error",
                "--fail",
                "--noproxy",
                "*",
                "--max-time",
                "5",
                "--url",
                &format!("http://{}/fixture", self.address),
            ]);
            stream_download(&spec, path, token, asset.size, &mut |done, total| {
                progress(done, total);
                if !self.overflow && done >= 64 * 1024 {
                    token.cancel();
                }
            })
        }
    }
    for overflow in [false, true] {
        let root = TestRoot::new();
        install(&root.0, ProvisionSelection::All, &FakeHost::default()).unwrap();
        let before = fs::read(root.0.join("current-tools.json")).unwrap();
        let count = fs::read_dir(root.0.join("generations")).unwrap().count();
        let mut bytes = pe();
        bytes.resize(2 * 1024 * 1024, 0xa5);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server_bytes = bytes.clone();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(7);
            let mut socket: TcpStream = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    other => panic!("loopback accept: {other:?}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request);
            write!(
                socket,
                "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n",
                server_bytes.len()
            )
            .unwrap();
            for chunk in server_bytes.chunks(8192) {
                if socket.write_all(chunk).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let parent = CancellationToken::default();
        let child = parent.child();
        let host = RawHost {
            address,
            base: FakeHost {
                ytdlp: if overflow { pe() } else { bytes },
                ..Default::default()
            },
            overflow,
        };
        let result = provision_at_with_progress(
            &root.0,
            ProvisionSelection::YtDlp,
            &child,
            101,
            |_| {},
            &host,
        );
        server.join().unwrap();
        assert!(result
            .unwrap_err()
            .contains(if overflow { "exceeded" } else { "cancel" }));
        assert_eq!(child.is_cancelled(), !overflow);
        assert!(!parent.is_cancelled());
        assert_eq!(fs::read(root.0.join("current-tools.json")).unwrap(), before);
        assert_eq!(
            fs::read_dir(root.0.join("generations")).unwrap().count(),
            count
        );
        let retry = parent.child();
        assert!(!retry.is_cancelled());
        provision_at_with_progress(
            &root.0,
            ProvisionSelection::YtDlp,
            &retry,
            102,
            |_| {},
            &FakeHost::default(),
        )
        .unwrap();
    }
}

#[cfg(unix)]
#[test]
fn symlinked_generation_root_and_manifest_files_cannot_escape_owned_root() {
    let root = TestRoot::new();
    let outside = TestRoot::new();
    fs::write(outside.0.join("sentinel"), b"keep").unwrap();
    std::os::unix::fs::symlink(&outside.0, root.0.join("generations")).unwrap();
    assert!(install(&root.0, ProvisionSelection::All, &FakeHost::default()).is_err());
    assert_eq!(fs::read(outside.0.join("sentinel")).unwrap(), b"keep");
    fs::remove_file(root.0.join("generations")).unwrap();
    install(&root.0, ProvisionSelection::All, &FakeHost::default()).unwrap();
    let manifest = load_manifest(&root.0).unwrap();
    let target = root.0.join(&manifest.tools[0].path);
    fs::remove_file(&target).unwrap();
    std::os::unix::fs::symlink(outside.0.join("sentinel"), &target).unwrap();
    assert!(load_manifest(&root.0).is_err());
}

#[test]
fn installer_download_plan_only_selects_missing_families() {
    assert_eq!(missing_selection(true, true), None);
    assert_eq!(
        missing_selection(false, true),
        Some(ProvisionSelection::YtDlp)
    );
    assert_eq!(
        missing_selection(true, false),
        Some(ProvisionSelection::FFmpeg)
    );
    assert_eq!(
        missing_selection(false, false),
        Some(ProvisionSelection::All)
    );
}

#[cfg(windows)]
#[test]
fn installer_reuses_existing_portable_tools_without_execution_or_network() {
    let root = TestRoot::new();
    fs::write(root.0.join("portable.txt"), b"").unwrap();
    let data = root.0.join("ytTrayDL-data");
    fs::create_dir(&data).unwrap();
    let yt = root.0.join("fixture-ytdlp.exe");
    let ff = root.0.join("fixture-ffmpeg.exe");
    fs::write(&yt, b"inert yt-dlp fixture").unwrap();
    fs::write(&ff, b"inert ffmpeg fixture").unwrap();
    let settings = crate::settings::Settings {
        ytdlp_path: yt.clone(),
        ffmpeg_path: ff.clone(),
        ..Default::default()
    };
    let settings_bytes = serde_json::to_vec(&settings).unwrap();
    fs::write(data.join("settings.json"), &settings_bytes).unwrap();
    let report = ensure_for_installer(&root.0).unwrap();
    assert_eq!(report.status, "success");
    assert!(report.tools.is_empty());
    assert!(!data.join("bin").exists());
    assert_eq!(
        fs::read(data.join("settings.json")).unwrap(),
        settings_bytes
    );
    assert_eq!(fs::read(yt).unwrap(), b"inert yt-dlp fixture");
    assert_eq!(fs::read(ff).unwrap(), b"inert ffmpeg fixture");
}
