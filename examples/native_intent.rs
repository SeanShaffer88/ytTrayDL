//! Owned two-process regression for the actual Windows platform URL receiver.
//! No GUI, downloads, clipboard writes, registry writes, or keyboard input.
#[cfg(windows)]
fn main() {
    use std::time::{Duration, Instant};
    use yttraydl::platform::{self, PlatformEvent};
    const URL: &str = "https://example.invalid/audit-cli";
    let sender = std::env::args().any(|arg| arg == "--sender");
    let setup = platform::setup(false);
    if sender {
        assert!(setup.already_running, "primary instance must already exist");
        platform::forward_manual_url(URL).expect("bounded URL delivery");
        return;
    }
    // The caller must also preflight the session identity before launch. Refuse
    // interference if another instance won the mutex in that interval.
    assert!(!setup.already_running, "existing instance; test refused");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--sender")
        .spawn()
        .expect("owned sender launch");
    let deadline = Instant::now() + Duration::from_secs(5);
    let received = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break None;
        }
        match setup.events.recv_timeout(remaining) {
            Ok(PlatformEvent::ManualUrl(url)) => break Some(url),
            Ok(_) => continue,
            Err(_) => break None,
        }
    };
    let status = loop {
        if let Some(status) = child.try_wait().expect("owned sender wait") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            // Do not perform an unbounded post-kill wait in the test process.
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(received.as_deref(), Some(URL), "exact receiver state");
    assert!(status.is_some_and(|status| status.success()), "sender exit");
    println!("{{\"status\":\"Pass\",\"exactUrl\":\"{URL}\",\"senderExitCode\":0}}");
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows native platform fixture; not executed on this target.");
}
