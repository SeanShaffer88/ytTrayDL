//! Owned two-process regression for the actual Windows platform intent receiver.
//! No GUI, downloads, clipboard writes, registry writes, or keyboard input.
#[cfg(windows)]
fn main() {
    use std::time::{Duration, Instant};
    use yttraydl::platform::{self, PlatformEvent};
    const URL: &str = "https://example.invalid/audit-cli";
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--sender") {
        let setup = platform::setup_with_launch_intent(false, true);
        assert!(setup.already_running, "primary instance must already exist");
        match args.get(2).map(String::as_str) {
            Some("url") => platform::forward_manual_url(URL),
            Some("downloads") => platform::forward_open_downloads(),
            Some("converter") => platform::forward_open_converter(),
            Some("tags") => platform::forward_open_tags(),
            Some("player") => platform::forward_open_player(),
            _ => panic!("unknown owned sender mode"),
        }
        .expect("bounded intent delivery");
        return;
    }
    // Refuse interference if another application instance owns the mutex.
    let setup = platform::setup_with_launch_intent(false, true);
    assert!(!setup.already_running, "existing instance; test refused");
    for (mode, expected) in [
        ("url", PlatformEvent::ManualUrl(URL.to_string())),
        ("downloads", PlatformEvent::OpenDownloads),
        ("converter", PlatformEvent::OpenConverter),
        ("tags", PlatformEvent::OpenTags),
        ("player", PlatformEvent::OpenPlayer),
        ("player", PlatformEvent::OpenPlayer),
    ] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--sender", mode])
            .spawn()
            .expect("owned sender launch");
        let deadline = Instant::now() + Duration::from_secs(5);
        let received = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break None;
            }
            match setup.events.recv_timeout(remaining) {
                Ok(PlatformEvent::ClipboardText(_)) => continue,
                Ok(event) => break Some(event),
                Err(_) => break None,
            }
        };
        let status = loop {
            if let Some(status) = child.try_wait().expect("owned sender wait") {
                break Some(status);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(received, Some(expected), "exact {mode} receiver state");
        assert!(
            status.is_some_and(|status| status.success()),
            "{mode} sender exit"
        );
    }
    println!("{{\"status\":\"Pass\",\"exactUrl\":\"{URL}\",\"senderExitCode\":0,\"modules\":[\"downloads\",\"converter\",\"tags\",\"player\",\"player\"]}}");
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows native platform fixture; not executed on this target.");
}
