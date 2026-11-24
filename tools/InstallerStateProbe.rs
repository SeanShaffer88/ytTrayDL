//! CI-only consumer of the actual library; never included in release payloads.
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--run-owned") {
        run_owned(&args[1..]);
        return;
    }
    if args.first().is_some_and(|a| a == "--tool-status") {
        let settings = yttraydl::settings::Settings::load();
        let ytdlp = yttraydl::tools::managed_ytdlp_path();
        let ffmpeg = yttraydl::tools::managed_ffmpeg_path();
        let located = yttraydl::ytdlp::locate_ytdlp(&settings, None)
            .expect("actual app resolver finds managed yt-dlp");
        assert_eq!(
            located.path, ytdlp,
            "app resolver consumes current manifest"
        );
        assert!(ytdlp.is_file() && ffmpeg.is_file());
        assert!(yttraydl::ytdlp::ffmpeg_available(&settings, None));
        let ffprobe = ffmpeg.with_file_name("ffprobe.exe");
        assert!(ffprobe.is_file());
        println!("Actual app resolvers consumed managed yt-dlp, ffmpeg and ffprobe");
        return;
    }
    let settings = yttraydl::settings::Settings::load();
    assert!(!settings.monitoring_enabled, "synthetic setting was lost");
    assert_eq!(settings.filename_template, "field-071-%(id)s.%(ext)s");
    let history = yttraydl::history::History::load();
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].title, "field-071-preserved");
    assert_eq!(history.entries[0].url, "https://example.invalid/field-071");
    assert_eq!(
        history.entries[0].status,
        yttraydl::history::HistoryStatus::Completed
    );
    assert_eq!(
        std::fs::read_to_string(yttraydl::settings::Settings::archive_path()).unwrap(),
        "generic field-071\n"
    );
    let windows = yttraydl::window_geometry::WindowGeometries::load();
    assert_eq!(windows.schema_version, 1);
    assert_eq!(
        windows.get(yttraydl::window_geometry::WindowKind::Main),
        Some(yttraydl::window_geometry::WindowGeometry {
            x: 120,
            y: 140,
            width: 800,
            height: 600,
            dpi: 96,
        })
    );
    let queue = yttraydl::player::load_queue(&yttraydl::player::saved_queue_path())
        .expect("saved player queue remains readable");
    assert_eq!(queue.items().len(), 1);
    assert_eq!(
        queue.items()[0].file_name().unwrap(),
        "field-071-missing.mp3"
    );
    let player = yttraydl::player::load_options(&yttraydl::player::player_options_path())
        .expect("saved player options remain readable");
    assert_eq!(player.ffplay_path, "field-071-player.exe");
    assert_eq!(player.volume, 47);
    assert!(player.repeat);
    println!("Settings, history, archive, window geometry and player sentinel assertions passed");
}

// Test tooling shares production pre-execution JobObject containment, deadlines,
// output limits, and verified whole-job termination. No target is loaded here.
fn run_owned(args: &[std::ffi::OsString]) {
    use yttraydl::process::{
        CancellationToken, OwnedProcess, ProcessError, ProcessLimits, ProcessSpec,
    };
    let seconds = owned_deadline(args.first()).expect("deadline must be 1..=1800 seconds");
    let program = args.get(1).expect("explicit program required");
    let process = OwnedProcess::spawn(&ProcessSpec::new(program).args(&args[2..]))
        .expect("owned process spawn failed");
    eprintln!(
        "ownedRootPid={} deadlineSeconds={seconds} containment=production-owned-tree",
        process.id()
    );
    let limits = ProcessLimits {
        timeout: std::time::Duration::from_secs(seconds),
        ..ProcessLimits::default()
    };
    match process.capture(limits, &CancellationToken::default()) {
        Ok(output) => {
            use std::io::Write;
            std::io::stdout().write_all(&output.stdout).unwrap();
            std::io::stderr().write_all(&output.stderr).unwrap();
            eprintln!(
                "ownedTreeCleanup=verified exitCode={:?}",
                output.status.code()
            );
            std::process::exit(output.status.code().unwrap_or(1));
        }
        Err(error) => {
            // Timeout is returned only after terminate_and_wait succeeds; I/O
            // cleanup failures retain their own error instead of claiming success.
            eprintln!("ownedProcessError={error:?}");
            let code = match error {
                ProcessError::Timeout => {
                    eprintln!("ownedTreeCleanup=verified timedOut=true");
                    124
                }
                ProcessError::Exit { code, .. } => {
                    eprintln!("ownedTreeCleanup=verified timedOut=false");
                    code.unwrap_or(1)
                }
                _ => 1,
            };
            std::process::exit(code);
        }
    }
}

fn owned_deadline(value: Option<&std::ffi::OsString>) -> Option<u64> {
    value
        .and_then(|value| value.to_str())
        .and_then(|value| value.parse().ok())
        .filter(|seconds| (1..=1800).contains(seconds))
}

#[cfg(test)]
mod tests {
    #[test]
    fn owned_deadlines_remain_explicit_and_bounded() {
        assert_eq!(super::owned_deadline(None), None);
        for invalid in ["0", "1801", "-1", "bad"] {
            assert_eq!(super::owned_deadline(Some(&invalid.into())), None);
        }
        for seconds in [1, 300, 1500, 1800] {
            assert_eq!(
                super::owned_deadline(Some(&seconds.to_string().into())),
                Some(seconds)
            );
        }
    }
}
