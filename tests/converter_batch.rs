use yttraydl::converter::run_batch;
use yttraydl::process::{CancellationToken, ProcessSpec};

fn fixture(mode: &str) -> ProcessSpec {
    ProcessSpec::new(env!("CARGO_BIN_EXE_process_fixture")).arg(mode)
}

#[test]
fn batch_runs_in_order_and_stops_after_failure() {
    let specs = [fixture("late"), fixture("exit"), fixture("late")];
    let mut observed = Vec::new();
    let error = run_batch(&specs, &CancellationToken::default(), |index, total| {
        observed.push((index, total));
    })
    .unwrap_err();
    assert_eq!(observed, [(1, 3), (2, 3)]);
    assert!(error.contains("file 2 of 3"));
}

#[test]
fn cancelled_batch_does_not_start_any_item() {
    let token = CancellationToken::default();
    token.cancel();
    let mut observed = Vec::new();
    let result = run_batch(&[fixture("late")], &token, |index, _| observed.push(index));
    assert!(result.unwrap_err().contains("cancelled"));
    assert!(observed.is_empty());
}

#[test]
fn running_batch_can_be_cancelled() {
    let token = CancellationToken::default();
    let worker_token = token.clone();
    let worker =
        std::thread::spawn(move || run_batch(&[fixture("hang")], &worker_token, |_, _| {}));
    std::thread::sleep(std::time::Duration::from_millis(50));
    token.cancel();
    assert!(worker.join().unwrap().unwrap_err().contains("cancel"));
}

#[test]
fn progress_stream_keeps_bounded_tail_instead_of_failing_on_large_output() {
    run_batch(
        &[fixture("stdoutcap")],
        &CancellationToken::default(),
        |_, _| {},
    )
    .unwrap();
}
