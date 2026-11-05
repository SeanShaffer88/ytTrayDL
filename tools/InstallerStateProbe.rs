//! CI-only consumer of the actual library; never included in release payloads.
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--run-owned") {
        run_owned(&args[1..]);
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
    println!("Settings, history, archive and window geometry sentinel assertions passed");
}

// Test tooling shares production pre-execution JobObject containment, deadlines,
// output limits, and verified whole-job termination. No target is loaded here.
fn run_owned(args: &[std::ffi::OsString]) {
    use yttraydl::process::{
        CancellationToken, OwnedProcess, ProcessError, ProcessLimits, ProcessSpec,
    };
    let seconds: u64 = args
        .first()
        .and_then(|s| s.to_str())
        .and_then(|s| s.parse().ok())
        .filter(|s| (1..=300).contains(s))
        .expect("deadline must be 1..=300 seconds");
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
