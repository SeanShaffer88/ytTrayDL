//! End-to-end test of the download pipeline on Unix: a fake
//! `yt-dlp` shell script emits real yt-dlp-shaped output, and the
//! DownloadManager must spawn it, parse its progress, and land the
//! job in the right final state.
//!
//! Gated to Unix because the fake is a shell script. On Windows the
//! same code path runs against real yt-dlp; the argument building
//! and line parsing it relies on are covered by unit tests.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};
use yttraydl::download::{DownloadEvent, DownloadManager, JobState};
use yttraydl::settings::{Preset, Settings};

fn write_fake_ytdlp(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    // Write to a temp name, sync, then rename into place: executing
    // a file that was just written in place can race the writer and
    // fail with ETXTBSY ("Text file busy") under parallel load —
    // this flaked the suite twice before the rename pattern.
    let tmp = dir.join("fake-yt-dlp.tmp");
    let path = dir.join("fake-yt-dlp");
    {
        let mut f = std::fs::File::create(&tmp).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f.sync_all().unwrap();
    }
    let mut perms = std::fs::metadata(&tmp).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&tmp, perms).unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    path
}

fn run_until_finished(
    manager: &mut DownloadManager,
    settings: &Settings,
    bin: &std::path::Path,
) -> yttraydl::download::Job {
    manager.pump(settings, Some(bin));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        for ev in manager.poll_events() {
            if let DownloadEvent::Finished(job) = ev {
                return job;
            }
        }
        manager.pump(settings, Some(bin));
        assert!(Instant::now() < deadline, "job never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_success_scenario(tag: &str) -> (yttraydl::download::Job, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("yttraydl-e2e-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out_file = dir.join("video.mp4");
    let script = format!(
        "echo '[download] Destination: {out}'\n\
         echo '[download]   0.0% of    1.00MiB at  1.00MiB/s ETA 00:01'\n\
         echo '[download]  50.0% of    1.00MiB at  1.00MiB/s ETA 00:00'\n\
         echo '[download] 100% of    1.00MiB in 00:00:01 at 1.00MiB/s'\n\
         printf 'fake' > '{out}'\n\
         exit 0\n",
        out = out_file.display()
    );
    let bin = write_fake_ytdlp(&dir, &script);

    let settings = Settings {
        download_folder: dir.clone(),
        max_concurrent_downloads: 1,
        ..Settings::default()
    };

    let mut manager = DownloadManager::new();
    manager.enqueue(
        "https://youtu.be/abc".into(),
        String::new(),
        Preset::Best,
        true,
    );
    // Let the just-renamed script settle before exec: this VM's
    // overlayfs has returned ETXTBSY ("Text file busy") for a
    // freshly written+renamed file under parallel load — three
    // flakes so far, always this test, always that error.
    std::thread::sleep(Duration::from_millis(250));
    let job = run_until_finished(&mut manager, &settings, &bin);
    (job, dir)
}

#[test]
fn successful_download_flows_through_the_manager() {
    // Retry ONLY the ETXTBSY harness artifact (see above); any
    // other failure is a real assertion failure on first sight.
    let mut job = None;
    for attempt in 0..3 {
        let (j, dir) = run_success_scenario(&attempt.to_string());
        let etxtbsy = j
            .error
            .as_deref()
            .is_some_and(|e| e.contains("Text file busy"));
        let out_file = dir.join("video.mp4");
        if j.state == JobState::Done {
            assert_eq!(j.percent, 100.0);
            assert_eq!(j.final_file.as_deref(), Some(out_file.as_path()));
            assert!(out_file.is_file());
        }
        let _ = std::fs::remove_dir_all(&dir);
        job = Some(j);
        if !etxtbsy {
            break;
        }
    }
    let job = job.unwrap();
    assert_eq!(
        job.state,
        JobState::Done,
        "error={:?} tail={:?}",
        job.error,
        job.tail_lines
    );
}

#[test]
fn failing_download_reports_its_error() {
    let dir = std::env::temp_dir().join(format!("yttraydl-e2e-fail-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = write_fake_ytdlp(&dir, "echo 'ERROR: Video unavailable' >&2\nexit 3\n");
    let settings = Settings {
        download_folder: dir.clone(),
        ..Settings::default()
    };

    let mut manager = DownloadManager::new();
    manager.enqueue(
        "https://youtu.be/gone".into(),
        String::new(),
        Preset::Best,
        true,
    );
    let job = run_until_finished(&mut manager, &settings, &bin);
    assert_eq!(job.state, JobState::Failed);
    assert!(
        job.error
            .as_deref()
            .unwrap_or("")
            .contains("Video unavailable")
            || job
                .tail_lines
                .iter()
                .any(|l| l.contains("Video unavailable")),
        "error should surface yt-dlp's message, got: {:?}",
        job.error
    );
    let _ = std::fs::remove_dir_all(&dir);
}
