use yttraydl::player::play;
use yttraydl::process::{CancellationToken, ProcessSpec};

fn fixture(mode: &str) -> ProcessSpec {
    ProcessSpec::new(env!("CARGO_BIN_EXE_process_fixture")).arg(mode)
}

#[test]
fn playback_process_reports_success_and_failure() {
    let token = CancellationToken::default();
    play(&fixture("late"), &token).unwrap();
    assert!(play(&fixture("exit"), &token).is_err());
}

#[test]
fn playback_can_be_cancelled_without_cancelling_another_job() {
    let parent = CancellationToken::default();
    let track = parent.child();
    let other = parent.child();
    let worker_token = track.clone();
    let worker = std::thread::spawn(move || play(&fixture("hang"), &worker_token));
    std::thread::sleep(std::time::Duration::from_millis(50));
    track.cancel();
    assert!(worker.join().unwrap().unwrap_err().contains("cancel"));
    assert!(!other.is_cancelled());
    assert!(play(&fixture("late"), &track)
        .unwrap_err()
        .contains("cancel"));
    play(&fixture("late"), &other).unwrap();
}
