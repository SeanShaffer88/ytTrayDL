//! Scratch timing harness for the performance audit (not part of
//! the shipped binary — an example target). Run with:
//!   cargo run --release --example perf_harness
//! Prints ns/op for the hot paths the audit identified, using only
//! the library's public API, plus a line-stream simulation that
//! counts how many yt-dlp output lines actually change what the
//! UI displays (the rest are pure event churn in `run_job`).

use std::hint::black_box;
use std::time::Instant;
use yttraydl::download::{Job, JobState};
use yttraydl::rules::decide;
use yttraydl::settings::{domain_matches, Settings};
use yttraydl::url_detect::{extract_urls, is_known_site, normalize_url};
use yttraydl::ytdlp::{parse_progress_line, ProgressEvent};

fn bench(name: &str, iters: u32, mut f: impl FnMut()) {
    // Warm up, then time.
    for _ in 0..(iters / 10).max(1) {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let elapsed = start.elapsed();
    println!(
        "{name:<44} {:>10.1} ns/op  ({} iters, {:?} total)",
        elapsed.as_nanos() as f64 / iters as f64,
        iters,
        elapsed
    );
}

fn realistic_settings() -> Settings {
    Settings {
        ignored_urls: vec![
            "https://youtu.be/never1".into(),
            "https://www.youtube.com/watch?v=never2".into(),
            "https://vimeo.com/never3".into(),
            "https://www.tiktok.com/@someone/video/never4".into(),
            "https://youtu.be/never5".into(),
        ],
        ignored_domains: vec![
            "spam.example".into(),
            "ads.example".into(),
            "tracker.example".into(),
        ],
        allowed_domains: vec!["myvideos.example".into(), "intranet.example".into()],
        auto_domains: vec!["vimeo.com".into(), "dailymotion.com".into()],
        ..Settings::default()
    }
}

fn main() {
    let s = realistic_settings();

    bench("decide() prompt path (youtube)", 20_000, || {
        black_box(decide(
            black_box("https://www.youtube.com/watch?v=abc123"),
            &s,
            false,
        ));
    });
    bench("decide() unknown host (full scan)", 20_000, || {
        black_box(decide(
            black_box("https://videos.example.org/watch/1"),
            &s,
            false,
        ));
    });
    bench("is_known_site(music.youtube.com)", 20_000, || {
        black_box(is_known_site(black_box("music.youtube.com")));
    });
    bench("is_known_site(unknown, full 69-scan)", 20_000, || {
        black_box(is_known_site(black_box("videos.example.org")));
    });
    bench("domain_matches(subdomain)", 20_000, || {
        black_box(domain_matches(
            black_box("music.youtube.com"),
            black_box("youtube.com"),
        ));
    });
    bench("normalize_url(typical)", 20_000, || {
        black_box(normalize_url(black_box(
            "HTTPS://YouTube.com:443/watch?v=AbC#t=10",
        )));
    });

    // A 10 KB chat log with a few links buried in prose.
    let mut text = String::new();
    for i in 0..90 {
        text.push_str(&format!(
            "person{i}: did you see that thing yesterday, it was wild lol\n"
        ));
    }
    text.push_str("watch https://www.youtube.com/watch?v=abc123 and https://vimeo.com/42 ok");
    bench("extract_urls(10 KB chat text)", 2_000, || {
        black_box(extract_urls(black_box(&text)));
    });

    bench("parse_progress_line(progress)", 50_000, || {
        black_box(parse_progress_line(black_box(
            "[download]  42.3% of  123.45MiB at  5.67MiB/s ETA 00:12",
        )));
    });
    bench("parse_progress_line(other)", 50_000, || {
        black_box(parse_progress_line(black_box(
            "[youtube] abc123: Downloading webpage",
        )));
    });

    // The per-line cost in run_job today: one full Job clone per
    // output line (tail_lines at its 30-line cap).
    let job = Job {
        id: 1,
        url: "https://www.youtube.com/watch?v=abc123".into(),
        title: "Some video title".into(),
        preset: yttraydl::settings::Preset::Best,
        state: JobState::Running,
        percent: 42.3,
        size_text: Some("123.45MiB".into()),
        speed_text: Some("5.67MiB/s".into()),
        eta_text: Some("00:12".into()),
        final_file: None,
        already_downloaded: false,
        use_archive: true,
        attempts: 0,
        playlist_all: None,
        output_dir: None,
        error: None,
        tail_lines: (0..30)
            .map(|i| format!("[download]  {i}.0% of  123.45MiB at  5.67MiB/s ETA 00:12"))
            .collect(),
    };
    bench("Job::clone (30 tail lines)", 50_000, || {
        black_box(job.clone());
    });

    // Line-stream simulation: how many lines of a realistic
    // yt-dlp session change anything the UI displays?
    let mut lines: Vec<String> = Vec::new();
    for tag in [
        "[youtube] abc: Downloading webpage",
        "[youtube] abc: Downloading ios player API JSON",
        "[info] abc: Downloading 1 format(s): 399+251",
        "WARNING: [youtube] Some warning text here",
        "[download] Destination: C:\\Videos\\Some Video.mp4",
    ] {
        lines.push(tag.to_string());
    }
    // Progress: each percent value printed twice (yt-dlp repeats
    // steady-state lines), speed/ETA text changing every 10%.
    for p in 0..100 {
        let eta = format!("00:{:02}", 59 - p / 2);
        for _ in 0..2 {
            lines.push(format!(
                "[download]  {p}.0% of  123.45MiB at  5.67MiB/s ETA {eta}"
            ));
        }
    }
    lines.push("[Merger] Merging formats into \"C:\\Videos\\Some Video.mp4\"".into());
    lines.push("[download] 100% of  123.45MiB in 00:00:23 at 5.67MiB/s".into());

    // (percent, total, speed, eta, placeholder for non-progress)
    type DisplayTuple = (f32, Option<String>, Option<String>, Option<String>, bool);
    let mut display_changes = 0usize;
    let mut last: Option<DisplayTuple> = None;
    for line in &lines {
        let changed = match parse_progress_line(line) {
            ProgressEvent::Progress {
                percent,
                total,
                speed,
                eta,
            } => {
                let cur = (percent, total, speed, eta, false);
                let c = last.as_ref() != Some(&cur);
                last = Some(cur);
                c
            }
            ProgressEvent::Other => false,
            _ => true,
        };
        if changed {
            display_changes += 1;
        }
    }
    println!(
        "line stream: {} lines total, {} change the displayed state ({} would be cloned+sent today)",
        lines.len(),
        display_changes,
        lines.len()
    );
}
