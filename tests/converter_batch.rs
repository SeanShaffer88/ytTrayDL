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
