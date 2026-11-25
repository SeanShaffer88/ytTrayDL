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

#[test]
fn failed_copy_falls_back_without_leaving_partial_copy_or_overwriting() {
    use yttraydl::converter::{
        prepare_conversion, run_prepared_batch, ConversionMethod, ConversionOptions,
        OptimizedProgress, TrimRange, VideoProfile,
    };
    let root = std::env::temp_dir().join(format!("yttraydl-copy-fallback-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let input = root.join("input.mp4");
    let output = root.join("output.mp4");
    std::fs::write(&input, b"input").unwrap();
    let fake = std::path::Path::new(env!("CARGO_BIN_EXE_process_fixture"));
    let options = ConversionOptions {
        trim: TrimRange::parse("1", "2").unwrap(),
        ..Default::default()
    };
    let job = prepare_conversion(fake, &input, &output, VideoProfile::Mp4, options).unwrap();
    let mut events = Vec::new();
    let methods = run_prepared_batch(&[job], Some(fake), &CancellationToken::default(), |event| {
        events.push(event)
    })
    .unwrap();
    assert_eq!(methods, [ConversionMethod::Reencode]);
    assert!(events.contains(&OptimizedProgress::Method {
        index: 1,
        total: 1,
        method: ConversionMethod::Reencode,
        fallback: true
    }));
    assert_eq!(std::fs::read(&output).unwrap(), b"reencoded");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_before_copy_starts_creates_no_output() {
    use yttraydl::converter::{
        prepare_conversion, run_prepared_batch, ConversionOptions, OptimizedProgress, TrimRange,
        VideoProfile,
    };
    let root = std::env::temp_dir().join(format!("yttraydl-copy-cancel-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let input = root.join("input.mp4");
    let output = root.join("output.mp4");
    std::fs::write(&input, b"input").unwrap();
    let fake = std::path::Path::new(env!("CARGO_BIN_EXE_process_fixture"));
    let options = ConversionOptions {
        trim: TrimRange::parse("1", "2").unwrap(),
        ..Default::default()
    };
    let job = prepare_conversion(fake, &input, &output, VideoProfile::Mp4, options).unwrap();
    let token = CancellationToken::default();
    let result = run_prepared_batch(&[job], Some(fake), &token, |event| {
        if matches!(event, OptimizedProgress::Method { .. }) {
            token.cancel();
        }
    });
    assert!(result.unwrap_err().contains("cancel"));
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}
