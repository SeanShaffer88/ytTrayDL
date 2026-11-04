use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};
use yttraydl::process::{run_capture, CancellationToken, ProcessLimits, ProcessSpec};
use yttraydl::{
    download::{DownloadManager, JobState},
    settings::{Preset, Settings},
};
fn fixture() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/audit-stage2");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join(format!("download_fixture{}", std::env::consts::EXE_SUFFIX));
        run_capture(
            &ProcessSpec::new("rustc")
                .args(["--edition=2021", "tests/fixtures/download_fixture.rs", "-o"])
                .arg(&bin),
            ProcessLimits::default(),
            &CancellationToken::default(),
        )
        .unwrap();
        bin
    })
    .as_path()
}
fn wait_for(manager: &mut DownloadManager, condition: impl Fn(&DownloadManager) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        manager.poll_events();
        if condition(manager) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "manager acknowledgement timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn start(mode: String) -> (DownloadManager, u64) {
    let mut manager = DownloadManager::new();
    let id = manager.enqueue(mode, "fixture".into(), Preset::Best, false);
    let settings = Settings {
        low_disk_threshold_mb: 0,
        ..Settings::default()
    };
    manager.pump(&settings, Some(fixture()));
    (manager, id)
}
#[test]
fn final_paths_are_applied_before_immediate_exit_acknowledgement() {
    for (mode, expected) in [("merge", "video.mp4"), ("extract", "video.mp3")] {
        let (mut manager, _) = start(mode.into());
        wait_for(&mut manager, |m| !m.finished_jobs().is_empty());
        let job = &manager.finished_jobs()[0];
        assert_eq!(job.state, JobState::Done, "{:?}", job.error);
        assert_eq!(job.final_file.as_deref(), Some(Path::new(expected)));
    }
}
#[test]
fn manager_pause_cancel_and_shutdown_stop_descendant_writes() {
    for action in ["pause", "cancel", "shutdown"] {
        let path =
            std::env::temp_dir().join(format!("yttraydl-manager-{}-{action}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let (mut manager, id) = start(format!("tree:{}", path.display()));
        wait_for(&mut manager, |m| {
            m.running_jobs()
                .any(|j| j.tail_lines.iter().any(|l| l == "READY"))
                || path.exists()
        });
        match action {
            "pause" => {
                manager.pause(id);
                wait_for(&mut manager, |m| m.paused_count() == 1);
            }
            "cancel" => {
                manager.cancel(id);
                wait_for(&mut manager, |m| !m.finished_jobs().is_empty());
                assert_eq!(manager.finished_jobs()[0].state, JobState::Cancelled);
            }
            _ => manager.shutdown(),
        }
        let size = std::fs::metadata(&path).unwrap().len();
        // Observation window after acknowledgement detects a still-writing descendant.
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), size, "{action}");
        if action == "pause" {
            assert!(manager.resume(id));
            manager.pump(
                &Settings {
                    low_disk_threshold_mb: 0,
                    ..Settings::default()
                },
                Some(fixture()),
            );
            wait_for(&mut manager, |_| {
                std::fs::metadata(&path).unwrap().len() > size
            });
            manager.cancel(id);
            wait_for(&mut manager, |m| !m.finished_jobs().is_empty());
            let resumed_size = std::fs::metadata(&path).unwrap().len();
            std::thread::sleep(Duration::from_millis(60));
            assert_eq!(std::fs::metadata(&path).unwrap().len(), resumed_size);
        }
        manager.shutdown();
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn stalled_subscription_releases_worker_for_healthy_subscription() {
    let settings = Settings::default();
    let token = CancellationToken::default();
    let bin = fixture();
    let started = Instant::now();
    let results: Vec<_> = ["probe-hang", "probe-healthy"]
        .iter()
        .map(|url| {
            let limits = ProcessLimits {
                timeout: if *url == "probe-hang" {
                    Duration::from_millis(100)
                } else {
                    Duration::from_secs(3)
                },
                ..ProcessLimits::default()
            };
            yttraydl::fetch_playlist_entries_with_limits(bin, &settings, url, 10, &token, limits)
        })
        .collect();
    assert!(results[0].is_empty());
    assert_eq!(results[1].len(), 1);
    assert_eq!(results[1][0].id, "healthy");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn background_shutdown_cancels_and_joins_owned_probe() {
    let tasks = yttraydl::BackgroundTasks::default();
    let bin = fixture().to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    let (release_cleanup, cleanup_released) = std::sync::mpsc::channel();
    tasks.spawn(move |token| {
        tx.send("started").unwrap();
        let result = run_capture(
            &ProcessSpec::new(bin).arg("probe-hang"),
            ProcessLimits::default(),
            &token,
        );
        assert!(matches!(
            result,
            Err(yttraydl::process::ProcessError::Cancelled)
        ));
        tx.send("stopped").unwrap();
        cleanup_released
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "started");
    std::thread::scope(|scope| {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let tasks = &tasks;
        scope.spawn(move || {
            tasks.shutdown();
            done_tx.send(()).unwrap();
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "stopped");
        let premature = done_rx.recv_timeout(Duration::from_millis(100));
        release_cleanup.send(()).unwrap();
        assert!(matches!(
            premature,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    tasks.spawn(|_| panic!("shutdown must prevent new work"));
}
