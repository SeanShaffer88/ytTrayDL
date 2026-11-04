use yttraydl::process::{run_capture, CancellationToken, ProcessLimits, ProcessSpec};
#[test]
fn cancellation_before_spawn() {
    let token = CancellationToken::default();
    token.cancel();
    let result = run_capture(
        &ProcessSpec::new("missing-owned-fixture"),
        ProcessLimits::default(),
        &token,
    );
    assert!(matches!(
        result,
        Err(yttraydl::process::ProcessError::Cancelled)
    ));
}

fn fixture(mode: &str) -> ProcessSpec {
    ProcessSpec::new(env!("CARGO_BIN_EXE_process_fixture")).arg(mode)
}
#[test]
fn exact_argv_and_final_output() {
    let values = [
        "space here",
        "",
        "quoted\"text",
        "& | %PATH%",
        "日本語",
        "{nested}",
    ];
    let output = run_capture(
        &fixture("argv").args(values),
        ProcessLimits::default(),
        &CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&output.stdout).unwrap(),
        values
    );
    let output = run_capture(
        &fixture("late"),
        ProcessLimits::default(),
        &CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(output.stdout, b"final output\n");
    assert_eq!(output.stderr, b"diagnostic\n");
}
#[test]
fn hangs_timeout_and_floods_are_bounded() {
    use yttraydl::process::ProcessError;
    let limits = ProcessLimits {
        timeout: std::time::Duration::from_millis(150),
        ..ProcessLimits::default()
    };
    assert!(matches!(
        run_capture(&fixture("hang"), limits, &CancellationToken::default()),
        Err(ProcessError::Timeout)
    ));
    let limits = ProcessLimits {
        stdout_bytes: 4096,
        stderr_bytes: 4096,
        line_bytes: 2048,
        ..ProcessLimits::default()
    };
    assert!(matches!(
        run_capture(&fixture("flood"), limits, &CancellationToken::default()),
        Err(ProcessError::OutputLimit { .. })
    ));
}
#[test]
fn cancellation_stops_grandchild_before_acknowledgement() {
    use yttraydl::process::{OutputStream, OwnedProcess, ProcessError};
    let path = std::env::temp_dir().join(format!(
        "yttraydl-owned-tree-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let token = CancellationToken::default();
    let cancel = token.clone();
    let mut ready = false;
    let result = OwnedProcess::spawn(&fixture("tree").arg(&path))
        .unwrap()
        .stream(ProcessLimits::default(), &token, |stream, chunk| {
            if stream == OutputStream::Stdout && chunk.windows(5).any(|s| s == b"READY") {
                ready = true;
                cancel.cancel();
            }
        });
    assert!(ready);
    assert!(matches!(result, Err(ProcessError::Cancelled)));
    let length = std::fs::metadata(&path).unwrap().len();
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), length);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn streaming_has_bounded_tails_without_lifetime_cap() {
    use yttraydl::process::OwnedProcess;
    let limits = ProcessLimits {
        stdout_bytes: 32,
        stderr_bytes: 16,
        line_bytes: 64,
        ..ProcessLimits::default()
    };
    let mut received = 0;
    let output = OwnedProcess::spawn(&fixture("lines"))
        .unwrap()
        .stream(limits, &CancellationToken::default(), |_, chunk| {
            received += chunk.len()
        })
        .unwrap();
    assert!(received > 20_000);
    assert_eq!(output.stdout.len(), 32);
    assert_eq!(output.stderr.len(), 16);
}
#[test]
fn capture_total_limit_and_exit_errors_are_typed() {
    use yttraydl::process::ProcessError;
    let limits = ProcessLimits {
        stdout_bytes: 64,
        stderr_bytes: 64,
        line_bytes: 1024,
        ..ProcessLimits::default()
    };
    assert!(matches!(
        run_capture(&fixture("lines"), limits, &CancellationToken::default()),
        Err(ProcessError::OutputLimit { .. })
    ));
    assert!(matches!(
        run_capture(
            &fixture("exit"),
            ProcessLimits::default(),
            &CancellationToken::default()
        ),
        Err(ProcessError::Exit { code: Some(7), .. })
    ));
    assert!(matches!(
        run_capture(
            &ProcessSpec::new("missing-owned-fixture"),
            ProcessLimits::default(),
            &CancellationToken::default()
        ),
        Err(ProcessError::Spawn(_))
    ));
}
#[test]
fn inherited_pipes_close_when_root_exits() {
    use yttraydl::process::OwnedProcess;
    let path = private_path("orphan");
    let output = OwnedProcess::spawn(&fixture("orphan").arg(&path))
        .unwrap()
        .capture(ProcessLimits::default(), &CancellationToken::default())
        .unwrap();
    assert_eq!(output.stdout, b"root exiting\n");
    assert_stopped_and_remove(&path);
}
fn private_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "yttraydl-{label}-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
fn assert_stopped_and_remove(path: &std::path::Path) {
    let length = std::fs::metadata(path).unwrap().len();
    std::thread::sleep(std::time::Duration::from_millis(75));
    assert_eq!(std::fs::metadata(path).unwrap().len(), length);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn timeout_and_explicit_stop_reap_trees_repeatedly() {
    use yttraydl::process::{OwnedProcess, ProcessError};
    let path = private_path("timeout");
    let limits = ProcessLimits {
        timeout: std::time::Duration::from_secs(2),
        ..ProcessLimits::default()
    };
    assert!(matches!(
        OwnedProcess::spawn(&fixture("tree").arg(&path))
            .unwrap()
            .capture(limits, &CancellationToken::default()),
        Err(ProcessError::Timeout)
    ));
    assert_stopped_and_remove(&path);
    for _ in 0..3 {
        let path = private_path("stop");
        let mut owner = OwnedProcess::spawn(&fixture("tree").arg(&path)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !path.exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        owner.terminate_and_wait().unwrap();
        assert!(owner.try_wait().unwrap().is_some());
        assert_stopped_and_remove(&path);
    }
}

#[test]
fn owner_drop_stops_tree_and_callback_failure_cleans_readers() {
    use yttraydl::process::{OwnedProcess, ProcessError};
    let path = private_path("drop");
    let owner = OwnedProcess::spawn(&fixture("tree").arg(&path)).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !path.exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    drop(owner);
    assert_stopped_and_remove(&path);
    let result = OwnedProcess::spawn(&fixture("lines")).unwrap().stream(
        ProcessLimits::default(),
        &CancellationToken::default(),
        |_, _| panic!("controlled callback failure"),
    );
    assert!(matches!(result, Err(ProcessError::Io(_))));
}

#[test]
fn capture_caps_are_enforced_independently() {
    use yttraydl::process::{OutputStream, ProcessError};
    for (mode, stream) in [
        ("stdoutcap", OutputStream::Stdout),
        ("stderrcap", OutputStream::Stderr),
    ] {
        let limits = ProcessLimits {
            stdout_bytes: 128,
            stderr_bytes: 128,
            line_bytes: 1024,
            ..ProcessLimits::default()
        };
        assert!(
            matches!(run_capture(&fixture(mode), limits, &CancellationToken::default()),
            Err(ProcessError::OutputLimit { stream: actual }) if actual == stream)
        );
    }
}

#[cfg(unix)]
#[test]
fn cancellation_with_escaped_inherited_pipe_is_bounded() {
    use yttraydl::process::{OutputStream, OwnedProcess, ProcessError};
    let path = private_path("escapedpipe");
    let token = CancellationToken::default();
    let cancel = token.clone();
    let mut cancelled_at = None;
    let result = OwnedProcess::spawn(&fixture("escapedpipe").arg(&path))
        .unwrap()
        .stream(ProcessLimits::default(), &token, |stream, bytes| {
            if stream == OutputStream::Stdout && bytes.windows(5).any(|s| s == b"READY") {
                cancelled_at = Some(std::time::Instant::now());
                cancel.cancel();
            }
        });
    assert!(matches!(result, Err(ProcessError::Cancelled)));
    assert!(cancelled_at.unwrap().elapsed() < std::time::Duration::from_secs(1));
    let done = std::path::PathBuf::from(format!("{}.done", path.display()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !done.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "escaped fixture did not self-clean"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    std::fs::remove_file(done).unwrap();
    std::fs::remove_file(path).unwrap();
}
