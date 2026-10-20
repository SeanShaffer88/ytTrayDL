//! The download manager: a small queue of yt-dlp child processes
//! with progress fan-out.
//!
//! Each job runs on its own thread, streaming the child's merged
//! stdout/stderr lines through `parse_progress_line` and reporting
//! state changes on a channel the UI drains each frame. Cancelling
//! kills the child process (yt-dlp keeps the `.part` file, and
//! `--continue` resumes it next time). At most
//! `max_concurrent_downloads` jobs run at once; the rest wait.

use crate::settings::{Preset, Settings};
use crate::ytdlp::{build_download_args, parse_progress_line, ProgressEvent};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    PostProcessing,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub preset: Preset,
    pub state: JobState,
    pub percent: f32,
    pub size_text: Option<String>,
    pub speed_text: Option<String>,
    pub eta_text: Option<String>,
    pub final_file: Option<PathBuf>,
    /// True when yt-dlp reported the file was already fully present.
    pub already_downloaded: bool,
    /// Whether this job consults the download archive (false for an
    /// explicit "download anyway" request).
    pub use_archive: bool,
    pub error: Option<String>,
    /// Last few output lines, kept for the error tooltip/details.
    pub tail_lines: Vec<String>,
}

impl Job {
    pub fn is_active(&self) -> bool {
        matches!(
            self.state,
            JobState::Queued | JobState::Running | JobState::PostProcessing
        )
    }
    pub fn status_text(&self) -> String {
        match self.state {
            JobState::Queued => "Queued".into(),
            JobState::Running => match (self.percent > 0.0, &self.eta_text) {
                (true, Some(eta)) => format!("Downloading — {:.0}% · ETA {eta}", self.percent),
                (true, None) => format!("Downloading — {:.0}%", self.percent),
                _ => "Downloading…".into(),
            },
            JobState::PostProcessing => "Processing (merging / converting)…".into(),
            JobState::Done => "Done".into(),
            JobState::Failed => format!(
                "Failed{}",
                self.error
                    .as_ref()
                    .map(|e| format!(" — {e}"))
                    .unwrap_or_default()
            ),
            JobState::Cancelled => "Cancelled".into(),
        }
    }
}

/// Events sent from job threads to the UI.
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Updated(Job),
    Finished(Job),
}

struct RunningJob {
    job: Job,
    cancel: Arc<AtomicBool>,
}

pub struct DownloadManager {
    next_id: u64,
    queue: VecDeque<Job>,
    running: Vec<RunningJob>,
    /// Recently finished jobs, kept for the Downloads window.
    finished: Vec<Job>,
    tx: Sender<DownloadEvent>,
    pub rx: Receiver<DownloadEvent>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadManager {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            next_id: 1,
            queue: VecDeque::new(),
            running: Vec::new(),
            finished: Vec::new(),
            tx,
            rx,
        }
    }

    pub fn enqueue(
        &mut self,
        url: String,
        title: String,
        preset: Preset,
        use_archive: bool,
    ) -> u64 {
        // De-duplicate identical in-flight URLs: a second request for
        // the same URL+preset just returns the existing job id.
        for r in &self.running {
            if r.job.url == url && r.job.preset == preset {
                return r.job.id;
            }
        }
        for q in &self.queue {
            if q.url == url && q.preset == preset {
                return q.id;
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.queue.push_back(Job {
            id,
            url,
            title,
            preset,
            state: JobState::Queued,
            percent: 0.0,
            size_text: None,
            speed_text: None,
            eta_text: None,
            final_file: None,
            already_downloaded: false,
            use_archive,
            error: None,
            tail_lines: Vec::new(),
        });
        id
    }

    /// Start queued jobs up to the concurrency limit. Call after
    /// enqueue and whenever a job finishes. `bin` is the resolved
    /// yt-dlp path; jobs stay queued when it is `None`.
    pub fn pump(&mut self, settings: &Settings, bin: Option<&std::path::Path>) {
        let Some(bin) = bin else { return };
        while self.running.len() < settings.max_concurrent_downloads.max(1) {
            let Some(mut job) = self.queue.pop_front() else {
                break;
            };
            job.state = JobState::Running;
            let cancel = Arc::new(AtomicBool::new(false));
            let tx = self.tx.clone();
            let bin = bin.to_path_buf();
            let args = build_download_args(settings, job.preset, &job.url, job.use_archive);
            let shared = Arc::new(Mutex::new(job.clone()));
            let thread_shared = shared;
            let thread_cancel = cancel.clone();
            std::thread::spawn(move || {
                run_job(bin, args, thread_shared, thread_cancel, tx);
            });
            self.running.push(RunningJob { job, cancel });
        }
    }

    /// Drain pending events into local state. Returns true when
    /// anything changed (so the UI can update tray text, history…).
    pub fn poll_events(&mut self) -> Vec<DownloadEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            match &ev {
                DownloadEvent::Updated(job) => {
                    if let Some(r) = self.running.iter_mut().find(|r| r.job.id == job.id) {
                        r.job = job.clone();
                    }
                }
                DownloadEvent::Finished(job) => {
                    self.running.retain(|r| r.job.id != job.id);
                    self.finished.insert(0, job.clone());
                    self.finished.truncate(50);
                }
            }
            events.push(ev);
        }
        events
    }

    pub fn cancel(&mut self, id: u64) {
        if let Some(pos) = self.queue.iter().position(|j| j.id == id) {
            let mut job = self.queue.remove(pos).unwrap();
            job.state = JobState::Cancelled;
            self.finished.insert(0, job);
            return;
        }
        if let Some(r) = self.running.iter().find(|r| r.job.id == id) {
            r.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Cancel everything (app exit): queued jobs are dropped,
    /// running jobs get their cancel flag — their workers kill
    /// the yt-dlp child within ~120 ms.
    pub fn cancel_all(&mut self) {
        self.queue.clear();
        for r in &self.running {
            r.cancel.store(true, Ordering::SeqCst);
        }
    }

    pub fn active_count(&self) -> usize {
        self.queue.len() + self.running.len()
    }

    pub fn running_jobs(&self) -> impl Iterator<Item = &Job> {
        self.running.iter().map(|r| &r.job)
    }
    pub fn queued_jobs(&self) -> impl Iterator<Item = &Job> {
        self.queue.iter()
    }
    pub fn finished_jobs(&self) -> &[Job] {
        &self.finished
    }

    /// An overall tray tooltip line, e.g. "ytTrayDL — downloading 1 (42%)".
    pub fn summary(&self) -> String {
        let active = self.active_count();
        if active == 0 {
            return "ytTrayDL — monitoring clipboard".into();
        }
        let best = self
            .running
            .iter()
            .map(|r| r.job.percent)
            .fold(0.0f32, f32::max);
        if self.running.is_empty() {
            format!("ytTrayDL — {active} queued")
        } else if best > 0.0 {
            format!("ytTrayDL — downloading {active} · {best:.0}%")
        } else {
            format!("ytTrayDL — downloading {active}")
        }
    }
}

fn run_job(
    bin: PathBuf,
    args: Vec<String>,
    shared: Arc<Mutex<Job>>,
    cancel: Arc<AtomicBool>,
    tx: Sender<DownloadEvent>,
) {
    let send_update = |job: &Job| {
        let _ = tx.send(DownloadEvent::Updated(job.clone()));
    };

    let spawned = Command::new(&bin)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child: Child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let mut job = shared.lock().unwrap();
            job.state = JobState::Failed;
            job.error = Some(format!("could not start yt-dlp ({}): {e}", bin.display()));
            let done = job.clone();
            drop(job);
            let _ = tx.send(DownloadEvent::Finished(done));
            return;
        }
    };

    // yt-dlp prints progress to stdout with --newline, errors and
    // some progress to stderr. Merge by reading stderr on a helper
    // thread into the same line channel.
    let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
    if let Some(out) = child.stdout.take() {
        let ltx = line_tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if ltx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let ltx = line_tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if ltx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    drop(line_tx);

    let mut last_destination: Option<PathBuf> = None;
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            let mut job = shared.lock().unwrap();
            job.state = JobState::Cancelled;
            let done = job.clone();
            drop(job);
            let _ = tx.send(DownloadEvent::Finished(done));
            return;
        }
        match line_rx.recv_timeout(std::time::Duration::from_millis(120)) {
            Ok(line) => {
                let mut job = shared.lock().unwrap();
                match parse_progress_line(&line) {
                    ProgressEvent::Destination(p) => {
                        last_destination = Some(p.clone());
                        job.final_file = Some(p);
                        if job.title.is_empty() || job.title == job.url {
                            if let Some(name) = last_destination
                                .as_ref()
                                .and_then(|p| p.file_stem())
                                .map(|s| s.to_string_lossy().to_string())
                            {
                                job.title = name;
                            }
                        }
                    }
                    ProgressEvent::Progress {
                        percent,
                        total,
                        speed,
                        eta,
                    } => {
                        job.state = JobState::Running;
                        job.percent = percent;
                        if total.is_some() {
                            job.size_text = total;
                        }
                        job.speed_text = speed;
                        job.eta_text = eta;
                    }
                    ProgressEvent::AlreadyDownloaded => {
                        job.percent = 100.0;
                        job.already_downloaded = true;
                    }
                    ProgressEvent::PostProcessing(desc) => {
                        job.state = JobState::PostProcessing;
                        // "[Merger] Merging formats into "X"" carries
                        // the final path; extract it when present.
                        if let Some(p) = desc
                            .split('"')
                            .nth(1)
                            .filter(|s| s.contains('.') && s.len() > 4)
                        {
                            job.final_file = Some(PathBuf::from(p));
                        }
                    }
                    ProgressEvent::Other => {}
                }
                if line.contains("ERROR") {
                    job.error = Some(line.trim().trim_start_matches("ERROR:").trim().to_string());
                }
                job.tail_lines.push(line);
                if job.tail_lines.len() > 30 {
                    job.tail_lines.remove(0);
                }
                let snap = job.clone();
                drop(job);
                send_update(&snap);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if let Ok(Some(_)) = child.try_wait() {
            // Give the reader threads a moment to flush final lines.
            std::thread::sleep(std::time::Duration::from_millis(60));
            break;
        }
    }

    // Drain any lines that arrived during the flush window.
    while let Ok(line) = line_rx.try_recv() {
        let mut job = shared.lock().unwrap();
        if let ProgressEvent::Destination(p) = parse_progress_line(&line) {
            job.final_file = Some(p);
        }
        job.tail_lines.push(line);
    }

    let status = child.wait();
    let mut job = shared.lock().unwrap();
    let cancelled = cancel.load(Ordering::SeqCst);
    match status {
        Ok(s) if s.success() && !cancelled => {
            job.state = JobState::Done;
            job.percent = 100.0;
            if job.final_file.is_none() {
                job.final_file = last_destination;
            }
        }
        Ok(_) if cancelled => {
            job.state = JobState::Cancelled;
        }
        Ok(s) => {
            job.state = JobState::Failed;
            if job.error.is_none() {
                job.error = Some(format!("yt-dlp exited with status {s}"));
            }
        }
        Err(e) => {
            job.state = JobState::Failed;
            job.error = Some(format!("waiting on yt-dlp failed: {e}"));
        }
    }
    let done = job.clone();
    drop(job);
    let _ = tx.send(DownloadEvent::Finished(done));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_dedupes_identical_jobs() {
        let mut m = DownloadManager::new();
        let a = m.enqueue("https://youtu.be/x".into(), "t".into(), Preset::Best, true);
        let b = m.enqueue("https://youtu.be/x".into(), "t".into(), Preset::Best, true);
        assert_eq!(a, b);
        let c = m.enqueue("https://youtu.be/x".into(), "t".into(), Preset::P720, true);
        assert_ne!(a, c);
        assert_eq!(m.active_count(), 2);
    }

    #[test]
    fn cancel_queued_job_moves_it_to_finished() {
        let mut m = DownloadManager::new();
        let id = m.enqueue("https://youtu.be/x".into(), "t".into(), Preset::Best, true);
        m.cancel(id);
        assert_eq!(m.active_count(), 0);
        assert_eq!(m.finished_jobs()[0].state, JobState::Cancelled);
    }

    #[test]
    fn summary_reflects_queue() {
        let mut m = DownloadManager::new();
        assert!(m.summary().contains("monitoring"));
        m.enqueue("https://youtu.be/x".into(), "t".into(), Preset::Best, true);
        assert!(m.summary().contains("queued"));
    }

    #[test]
    fn status_text_variants() {
        let mut j = Job {
            id: 1,
            url: "u".into(),
            title: "t".into(),
            preset: Preset::Best,
            state: JobState::Running,
            percent: 42.0,
            size_text: None,
            speed_text: None,
            eta_text: Some("00:12".into()),
            final_file: None,
            already_downloaded: false,
            use_archive: true,
            error: None,
            tail_lines: Vec::new(),
        };
        assert!(j.status_text().contains("42%"));
        j.state = JobState::Done;
        assert_eq!(j.status_text(), "Done");
    }
}
