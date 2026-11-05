//! The download manager: a small queue of yt-dlp child processes
//! with progress fan-out.
//!
//! Each job runs on its own thread, streaming the child's merged
//! stdout/stderr lines through `parse_progress_line` and reporting
//! state changes on a channel the UI drains each frame. Cancelling
//! kills the child process (yt-dlp keeps the `.part` file, and
//! `--continue` resumes it next time). At most
//! `max_concurrent_downloads` jobs run at once; the rest wait.

use crate::process::{
    CancellationToken, OutputStream, OwnedProcess, ProcessError, ProcessLimits, ProcessSpec,
};
use crate::settings::{Preset, Settings};
use crate::ytdlp::{
    build_download_args_full, effective_destination, parse_progress_line, ProgressEvent,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    PostProcessing,
    /// Stopped by the user with the partial file kept; a resume
    /// re-runs the job and yt-dlp's `--continue` picks up the
    /// `.part` file where it left off.
    Paused,
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
    /// How many times this job has already been retried
    /// (automatic retries + manual requeues start fresh at 0;
    /// the auto-retry path carries the count forward).
    pub attempts: u32,
    /// Per-download playlist choice from the popup: `Some(true)`
    /// = whole playlist, `Some(false)` = just the linked item,
    /// `None` = follow the global setting.
    pub playlist_all: Option<bool>,
    /// Per-download output folder from the popup; `None` = the
    /// settings download folder.
    pub output_dir: Option<PathBuf>,
    pub error: Option<String>,
    /// Last few output lines, kept for the error tooltip/details.
    pub tail_lines: Vec<String>,
}

impl Job {
    /// Next automatic retry number, only for a failed job within the budget.
    /// Pauses and cancellation are user intent and never consume retry budget.
    pub fn automatic_retry_attempt(&self, maximum: u32) -> Option<u32> {
        (self.state == JobState::Failed && self.attempts < maximum).then(|| self.attempts + 1)
    }

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
            JobState::Paused => "Paused".into(),
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
    pause: Arc<AtomicBool>,
    stop: CancellationToken,
    worker: std::thread::JoinHandle<()>,
}

pub struct DownloadManager {
    next_id: u64,
    queue: VecDeque<Job>,
    running: Vec<RunningJob>,
    /// Jobs the user paused; they hold no process and wait for
    /// an explicit resume.
    paused: Vec<Job>,
    /// Recently finished jobs, kept for the Downloads window.
    finished: Vec<Job>,
    tx: Sender<DownloadEvent>,
    pub rx: Receiver<DownloadEvent>,
    disk_blocked: Vec<(u64, Instant)>,
    shutting_down: bool,
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
            paused: Vec::new(),
            finished: Vec::new(),
            tx,
            rx,
            disk_blocked: Vec::new(),
            shutting_down: false,
        }
    }

    pub fn enqueue(
        &mut self,
        url: String,
        title: String,
        preset: Preset,
        use_archive: bool,
    ) -> u64 {
        self.enqueue_with_attempts(url, title, preset, use_archive, 0)
    }

    /// Enqueue with a carried-forward retry count (auto-retry).
    pub fn enqueue_with_attempts(
        &mut self,
        url: String,
        title: String,
        preset: Preset,
        use_archive: bool,
        attempts: u32,
    ) -> u64 {
        self.enqueue_full(url, title, preset, use_archive, attempts, None)
    }

    /// The full enqueue: retry count + per-download playlist
    /// choice. The other enqueue helpers delegate here.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_full(
        &mut self,
        url: String,
        title: String,
        preset: Preset,
        use_archive: bool,
        attempts: u32,
        playlist_all: Option<bool>,
    ) -> u64 {
        self.enqueue_job(
            url,
            title,
            preset,
            use_archive,
            attempts,
            playlist_all,
            None,
        )
    }

    /// The fullest enqueue: everything `enqueue_full` takes plus
    /// a per-download output folder.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_job(
        &mut self,
        url: String,
        title: String,
        preset: Preset,
        use_archive: bool,
        attempts: u32,
        playlist_all: Option<bool>,
        output_dir: Option<PathBuf>,
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
            attempts,
            playlist_all,
            output_dir,
            error: None,
            tail_lines: Vec::new(),
        });
        id
    }

    /// Start queued jobs up to the concurrency limit. Call after
    /// enqueue and whenever a job finishes. `bin` is the resolved
    /// yt-dlp path; jobs stay queued when it is `None`.
    pub fn pump(&mut self, settings: &Settings, bin: Option<&std::path::Path>) {
        self.pump_with_disk(settings, bin, Instant::now(), |path| {
            let mut probe = path.to_path_buf();
            while !probe.exists() && probe.pop() {}
            let free = crate::platform::free_disk_bytes(&probe);
            if free.is_none() {
                crate::diag_log(&format!(
                    "Disk space unknown for {}; allowing download",
                    path.display()
                ));
            }
            free
        });
    }

    /// Injected clock and disk query keep admission and recovery testable.
    pub fn pump_with_disk(
        &mut self,
        settings: &Settings,
        bin: Option<&std::path::Path>,
        now: Instant,
        mut free_bytes: impl FnMut(&std::path::Path) -> Option<u64>,
    ) {
        self.disk_blocked
            .retain(|(id, _)| self.queue.iter().any(|j| j.id == *id));
        if settings.low_disk_threshold_mb == 0 {
            self.disk_blocked.clear();
        }
        let Some(bin) = bin else {
            for (_, next) in &mut self.disk_blocked {
                if *next <= now {
                    *next = now + Duration::from_secs(2);
                }
            }
            return;
        };
        if self.shutting_down {
            return;
        }
        while self.running.len() < settings.max_concurrent_downloads.max(1) {
            let Some(pos) = self.queue.iter().position(|job| {
                if settings.low_disk_threshold_mb == 0 {
                    return true;
                }
                if self
                    .disk_blocked
                    .iter()
                    .any(|(id, next)| *id == job.id && now < *next)
                {
                    return false;
                }
                self.disk_blocked.retain(|(id, _)| *id != job.id);
                let path = effective_destination(settings, job.output_dir.as_deref());
                match free_bytes(path) {
                    Some(free)
                        if free < settings.low_disk_threshold_mb.saturating_mul(1024 * 1024) =>
                    {
                        self.disk_blocked
                            .push((job.id, now + Duration::from_secs(2)));
                        false
                    }
                    None => true,
                    Some(_) => true,
                }
            }) else {
                break;
            };
            let mut job = self.queue.remove(pos).unwrap();
            job.state = JobState::Running;
            let cancel = Arc::new(AtomicBool::new(false));
            let pause = Arc::new(AtomicBool::new(false));
            let tx = self.tx.clone();
            let bin = bin.to_path_buf();
            let args = build_download_args_full(
                settings,
                job.preset,
                &job.url,
                job.use_archive,
                job.playlist_all,
                job.output_dir.as_deref(),
            );
            let shared = Arc::new(Mutex::new(job.clone()));
            let thread_shared = shared;
            let thread_cancel = cancel.clone();
            let thread_pause = pause.clone();
            let stop = CancellationToken::default();
            let thread_stop = stop.clone();
            let worker = std::thread::spawn(move || {
                run_job(
                    bin,
                    args,
                    thread_shared,
                    thread_cancel,
                    thread_pause,
                    thread_stop,
                    tx,
                );
            });
            self.running.push(RunningJob {
                job,
                cancel,
                pause,
                stop,
                worker,
            });
        }
        // Occupied slots must not leave an expired retry requesting zero-delay
        // repaints while running downloads finish.
        if self.running.len() >= settings.max_concurrent_downloads.max(1) {
            for (_, next) in &mut self.disk_blocked {
                if *next <= now {
                    *next = now + Duration::from_secs(2);
                }
            }
        }
    }

    /// Remaining delay until blocked queued destinations need another check.
    pub fn disk_recheck_after(&self, now: Instant) -> Option<Duration> {
        self.disk_blocked
            .iter()
            .filter(|(id, _)| self.queue.iter().any(|j| j.id == *id))
            .map(|(_, next)| next.saturating_duration_since(now))
            .min()
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
                    if let Some(pos) = self.running.iter().position(|r| r.job.id == job.id) {
                        let _ = self.running.remove(pos).worker.join();
                    }
                    if job.state == JobState::Paused {
                        // A paused job is not finished — it waits
                        // in the paused list for an explicit resume.
                        self.paused.retain(|j| j.id != job.id);
                        self.paused.push(job.clone());
                    } else {
                        self.finished.insert(0, job.clone());
                        self.finished.truncate(50);
                    }
                }
            }
            events.push(ev);
        }
        events
    }

    /// Cancel and join every owned download before returning.
    pub fn shutdown(&mut self) {
        self.shutting_down = true;
        self.cancel_all();
        for running in self.running.drain(..) {
            if running.worker.join().is_err() {
                crate::diag_log("download shutdown: worker panicked");
            }
        }
    }

    pub fn cancel(&mut self, id: u64) {
        if let Some(pos) = self.queue.iter().position(|j| j.id == id) {
            let mut job = self.queue.remove(pos).unwrap();
            job.state = JobState::Cancelled;
            self.finished.insert(0, job);
            return;
        }
        if let Some(pos) = self.paused.iter().position(|j| j.id == id) {
            let mut job = self.paused.remove(pos);
            job.state = JobState::Cancelled;
            self.finished.insert(0, job);
            return;
        }
        if let Some(r) = self.running.iter().find(|r| r.job.id == id) {
            r.cancel.store(true, Ordering::SeqCst);
            r.stop.cancel();
        }
    }

    /// Pause a job: a queued job simply waits in the paused
    /// list; a running job's worker kills the yt-dlp child and
    /// reports back as Paused (the `.part` file is kept, so a
    /// later resume continues where it stopped).
    pub fn pause(&mut self, id: u64) {
        if let Some(pos) = self.queue.iter().position(|j| j.id == id) {
            let mut job = self.queue.remove(pos).unwrap();
            job.state = JobState::Paused;
            self.paused.push(job);
            return;
        }
        if let Some(r) = self.running.iter().find(|r| r.job.id == id) {
            r.pause.store(true, Ordering::SeqCst);
            r.stop.cancel();
        }
    }

    /// Resume a paused job: back into the queue (the caller
    /// pumps). Returns false when the id isn't paused.
    pub fn resume(&mut self, id: u64) -> bool {
        if let Some(pos) = self.paused.iter().position(|j| j.id == id) {
            let mut job = self.paused.remove(pos);
            job.state = JobState::Queued;
            self.queue.push_back(job);
            return true;
        }
        false
    }

    /// Pause everything pausable (queued + running).
    pub fn pause_all(&mut self) {
        while let Some(mut job) = self.queue.pop_front() {
            job.state = JobState::Paused;
            self.paused.push(job);
        }
        for r in &self.running {
            r.pause.store(true, Ordering::SeqCst);
            r.stop.cancel();
        }
    }

    /// Resume every paused job.
    pub fn resume_all(&mut self) {
        for mut job in self.paused.drain(..) {
            job.state = JobState::Queued;
            self.queue.push_back(job);
        }
    }

    /// Move a queued job one step toward the front (`up`) or
    /// the back of the queue. Returns false when it can't move
    /// (already at the end, running, or unknown id).
    pub fn move_queued(&mut self, id: u64, up: bool) -> bool {
        let Some(pos) = self.queue.iter().position(|j| j.id == id) else {
            return false;
        };
        if up {
            if pos == 0 {
                return false;
            }
            self.queue.swap(pos, pos - 1);
        } else {
            if pos + 1 >= self.queue.len() {
                return false;
            }
            self.queue.swap(pos, pos + 1);
        }
        true
    }

    /// Requeue a finished job with its original preset and
    /// archive choice (manual retry). The finished entry is
    /// removed; the new job starts fresh. Returns the new id.
    pub fn requeue_finished(&mut self, id: u64) -> Option<u64> {
        let pos = self.finished.iter().position(|j| j.id == id)?;
        let old = self.finished.remove(pos);
        Some(self.enqueue_job(
            old.url,
            old.title,
            old.preset,
            old.use_archive,
            0,
            old.playlist_all,
            old.output_dir,
        ))
    }

    /// Remove one finished entry from the list (files stay).
    pub fn remove_finished(&mut self, id: u64) {
        self.finished.retain(|j| j.id != id);
    }

    /// Clear the whole finished list (files stay).
    pub fn clear_finished(&mut self) {
        self.finished.clear();
    }

    /// Cancel everything: queued and paused jobs land in the
    /// finished list as Cancelled; running jobs get their
    /// cancel flag — their workers kill the yt-dlp child
    /// within ~120 ms. Used by the Downloads window's
    /// "Cancel all" and by app exit.
    pub fn cancel_all(&mut self) {
        while let Some(mut job) = self.queue.pop_front() {
            job.state = JobState::Cancelled;
            self.finished.insert(0, job);
        }
        for mut job in self.paused.drain(..) {
            job.state = JobState::Cancelled;
            self.finished.insert(0, job);
        }
        for r in &self.running {
            r.cancel.store(true, Ordering::SeqCst);
            r.stop.cancel();
        }
    }

    pub fn active_count(&self) -> usize {
        self.queue.len() + self.running.len()
    }

    pub fn paused_count(&self) -> usize {
        self.paused.len()
    }

    pub fn has_paused(&self) -> bool {
        !self.paused.is_empty()
    }

    pub fn running_jobs(&self) -> impl Iterator<Item = &Job> {
        self.running.iter().map(|r| &r.job)
    }
    pub fn queued_jobs(&self) -> impl Iterator<Item = &Job> {
        self.queue.iter()
    }
    pub fn paused_jobs(&self) -> &[Job] {
        &self.paused
    }
    pub fn finished_jobs(&self) -> &[Job] {
        &self.finished
    }

    /// An overall tray tooltip line, e.g. "ytTrayDL — downloading 1 (42%)".
    pub fn summary(&self) -> String {
        let active = self.active_count();
        if active == 0 {
            if self.paused_count() > 0 {
                return format!("ytTrayDL — {} paused", self.paused_count());
            }
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

/// The display-relevant snapshot of a job: every field the UI
/// renders (panel rows, Downloads window, tray text, taskbar).
/// The diagnostic tail is deliberately excluded — nothing shows
/// a running job's tail, and the Finished event always carries
/// the complete job. `run_job` keeps the last-sent snapshot so
/// an `Updated` event (a full `Job` clone + channel hop, plus
/// another clone when the UI applies it) is only paid when
/// something shown actually changed; most yt-dlp lines —
/// extractor chatter, repeated steady-state progress — change
/// nothing displayed (perf).
#[derive(PartialEq)]
struct DisplayState {
    state: JobState,
    percent: f32,
    title: String,
    size_text: Option<String>,
    speed_text: Option<String>,
    eta_text: Option<String>,
    final_file: Option<PathBuf>,
    error: Option<String>,
    already_downloaded: bool,
}

impl DisplayState {
    fn of(job: &Job) -> Self {
        Self {
            state: job.state,
            percent: job.percent,
            title: job.title.clone(),
            size_text: job.size_text.clone(),
            speed_text: job.speed_text.clone(),
            eta_text: job.eta_text.clone(),
            final_file: job.final_file.clone(),
            error: job.error.clone(),
            already_downloaded: job.already_downloaded,
        }
    }

    /// Field-by-field compare against a live job — no allocation,
    /// so the per-line check costs a few short string compares
    /// instead of a full `Job` clone.
    fn differs_from(&self, job: &Job) -> bool {
        self.state != job.state
            || self.percent != job.percent
            || self.title != job.title
            || self.size_text != job.size_text
            || self.speed_text != job.speed_text
            || self.eta_text != job.eta_text
            || self.final_file != job.final_file
            || self.error != job.error
            || self.already_downloaded != job.already_downloaded
    }
}

fn apply_output_line(job: &mut Job, line: String, authoritative_final: &mut bool) {
    match parse_progress_line(&line) {
        ProgressEvent::Destination(path) | ProgressEvent::ProcessedPath(path)
            if !*authoritative_final =>
        {
            if line.starts_with("[Merger]") || line.starts_with("[ExtractAudio]") {
                job.state = JobState::PostProcessing;
            }
            if job.title.is_empty() || job.title == job.url {
                if let Some(name) = path.file_stem() {
                    job.title = name.to_string_lossy().into_owned();
                }
            }
            job.final_file = Some(path);
        }
        ProgressEvent::FinalPath(path) => {
            *authoritative_final = true;
            job.final_file = Some(path);
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
        ProgressEvent::PostProcessing(_) => job.state = JobState::PostProcessing,
        _ => {}
    }
    if line.contains("ERROR") {
        job.error = Some(line.trim().trim_start_matches("ERROR:").trim().to_string());
    }
    job.tail_lines.push(line);
    if job.tail_lines.len() > 30 {
        job.tail_lines.remove(0);
    }
}

fn run_job(
    bin: PathBuf,
    args: Vec<String>,
    shared: Arc<Mutex<Job>>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    stop: CancellationToken,
    tx: Sender<DownloadEvent>,
) {
    let mut authoritative_final = false;
    let mut last_sent: Option<DisplayState> = None;
    let mut buffers = [Vec::<u8>::new(), Vec::<u8>::new()];
    let result = OwnedProcess::spawn(&ProcessSpec::new(&bin).args(&args)).and_then(|process| {
        process.stream(
            ProcessLimits {
                timeout: Duration::MAX,
                ..Default::default()
            },
            &stop,
            |stream, bytes| {
                let buffer = &mut buffers[if stream == OutputStream::Stdout { 0 } else { 1 }];
                buffer.extend_from_slice(bytes);
                while let Some(end) = buffer.iter().position(|b| *b == b'\n' || *b == b'\r') {
                    let line = String::from_utf8_lossy(&buffer[..end])
                        .trim_end_matches('\r')
                        .to_string();
                    buffer.drain(..=end);
                    let mut job = shared.lock().unwrap();
                    apply_output_line(&mut job, line, &mut authoritative_final);
                    if last_sent
                        .as_ref()
                        .is_none_or(|prev| prev.differs_from(&job))
                    {
                        last_sent = Some(DisplayState::of(&job));
                        let _ = tx.send(DownloadEvent::Updated(job.clone()));
                    }
                }
            },
        )
    });
    // stream returns only after tree termination and reader completion.
    for buffer in buffers {
        if !buffer.is_empty() {
            apply_output_line(
                &mut shared.lock().unwrap(),
                String::from_utf8_lossy(&buffer).into_owned(),
                &mut authoritative_final,
            );
        }
    }
    let mut job = shared.lock().unwrap();
    let cancelled = cancel.load(Ordering::SeqCst);
    let paused = pause.load(Ordering::SeqCst);
    match result {
        Ok(s) if s.status.success() && !cancelled && !paused => {
            job.state = JobState::Done;
            job.percent = 100.0;
        }
        Ok(_) | Err(ProcessError::Cancelled) if cancelled => {
            job.state = JobState::Cancelled;
        }
        Ok(_) | Err(ProcessError::Cancelled) if paused => {
            job.state = JobState::Paused;
        }
        Ok(s) => {
            job.state = JobState::Failed;
            if job.error.is_none() {
                job.error = Some(format!("yt-dlp exited with status {}", s.status));
            }
        }
        Err(e) => {
            job.state = JobState::Failed;
            crate::diag_log(&format!("download process cleanup/failure: {e}"));
            job.error = Some(format!("waiting on yt-dlp failed: {e}"));
        }
    }
    let done = job.clone();
    drop(job);
    let _ = tx.send(DownloadEvent::Finished(done));
}

impl Drop for DownloadManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_retry_only_requeues_failed_jobs_with_budget() {
        let mut manager = DownloadManager::new();
        manager.enqueue("url".into(), String::new(), Preset::Best, false);
        let mut job = manager.queue.pop_front().unwrap();
        for state in [
            JobState::Queued,
            JobState::Running,
            JobState::PostProcessing,
            JobState::Paused,
            JobState::Done,
            JobState::Cancelled,
        ] {
            job.state = state;
            assert_eq!(job.automatic_retry_attempt(2), None);
        }
        job.state = JobState::Failed;
        assert_eq!(job.automatic_retry_attempt(0), None);
        assert_eq!(job.automatic_retry_attempt(2), Some(1));
        job.attempts = 1;
        assert_eq!(job.automatic_retry_attempt(2), Some(2));
        job.attempts = 2;
        assert_eq!(job.automatic_retry_attempt(2), None);
        job.attempts = u32::MAX;
        assert_eq!(job.automatic_retry_attempt(u32::MAX), None);
    }

    #[test]
    fn final_output_survives_late_intermediates_and_tail_is_bounded() {
        let mut manager = DownloadManager::new();
        manager.enqueue("url".into(), String::new(), Preset::Best, false);
        let mut job = manager.queue.pop_front().unwrap();
        let mut authoritative = false;
        for line in [
            "[download] Destination: video.f137.mp4",
            "[download] Destination: video.f140.m4a",
            "[Merger] Merging formats into \"video.mp4\"",
        ] {
            apply_output_line(&mut job, line.into(), &mut authoritative);
        }
        assert_eq!(job.final_file, Some(PathBuf::from("video.mp4")));
        apply_output_line(
            &mut job,
            "[ExtractAudio] Destination: audio.mp3".into(),
            &mut authoritative,
        );
        assert_eq!(job.final_file, Some(PathBuf::from("audio.mp3")));
        apply_output_line(
            &mut job,
            "yttraydl:final:final.mp3".into(),
            &mut authoritative,
        );
        apply_output_line(
            &mut job,
            "[download] Destination: intermediate.m4a".into(),
            &mut authoritative,
        );
        assert_eq!(job.final_file, Some(PathBuf::from("final.mp3")));
        for _ in 0..100 {
            apply_output_line(&mut job, "chatter".into(), &mut authoritative);
        }
        assert_eq!(job.tail_lines.len(), 30);
    }

    #[test]
    fn disk_admission_checks_overrides_and_recovers_on_injected_clock() {
        let mut manager = DownloadManager::new();
        let settings = Settings {
            download_folder: "blocked".into(),
            low_disk_threshold_mb: 1,
            max_concurrent_downloads: 2,
            ..Settings::default()
        };
        let blocked = manager.enqueue("blocked-url".into(), String::new(), Preset::Best, false);
        let healthy = manager.enqueue_job(
            "healthy-url".into(),
            String::new(),
            Preset::Best,
            false,
            0,
            None,
            Some("healthy".into()),
        );
        let now = Instant::now();
        let nonexistent_bin = std::path::Path::new("fixture-does-not-exist");
        let mut queried = Vec::new();
        manager.pump_with_disk(&settings, Some(nonexistent_bin), now, |path| {
            queried.push(path.to_path_buf());
            Some(if path == std::path::Path::new("healthy") {
                u64::MAX
            } else {
                0
            })
        });
        assert_eq!(manager.running[0].job.id, healthy);
        assert_eq!(manager.queue[0].id, blocked);
        assert_eq!(
            queried,
            [PathBuf::from("blocked"), PathBuf::from("healthy")]
        );
        assert_eq!(
            manager.disk_recheck_after(now),
            Some(Duration::from_secs(2))
        );
        manager.pump_with_disk(
            &settings,
            Some(nonexistent_bin),
            now + Duration::from_secs(1),
            |_| panic!("checked disk too early"),
        );
        manager.pump_with_disk(
            &settings,
            Some(nonexistent_bin),
            now + Duration::from_secs(2),
            |_| Some(u64::MAX),
        );
        assert!(manager.queue.is_empty());
        assert_eq!(manager.disk_recheck_after(now), None);
    }

    #[test]
    fn missing_binary_defers_expired_disk_rechecks_and_recovers() {
        let mut manager = DownloadManager::new();
        let settings = Settings {
            low_disk_threshold_mb: 1,
            ..Settings::default()
        };
        let id = manager.enqueue("blocked".into(), String::new(), Preset::Best, false);
        let bin = std::path::Path::new("fixture-does-not-exist");
        let now = Instant::now();
        manager.pump_with_disk(&settings, Some(bin), now, |_| Some(0));
        assert_eq!(manager.queue[0].id, id);
        for seconds in [1, 2, 3, 4] {
            let check_at = now + Duration::from_secs(seconds);
            manager.pump_with_disk(&settings, None, check_at, |_| {
                panic!("disk queried without binary")
            });
            assert_eq!(
                manager.disk_recheck_after(check_at),
                Some(Duration::from_secs(if seconds % 2 == 0 { 2 } else { 1 }))
            );
            assert_eq!(manager.queue[0].id, id);
            assert!(manager.running.is_empty());
        }
        manager.pump_with_disk(&settings, Some(bin), now + Duration::from_secs(5), |_| {
            panic!("disk queried before deferred deadline")
        });
        manager.pump_with_disk(&settings, Some(bin), now + Duration::from_secs(6), |_| {
            Some(u64::MAX)
        });
        assert!(manager.queue.is_empty());
        assert_eq!(manager.running[0].job.id, id);
        assert_eq!(
            manager.disk_recheck_after(now + Duration::from_secs(6)),
            None
        );
    }

    #[test]
    fn disk_admission_disabled_or_unknown_allows_work() {
        for threshold in [0, 1] {
            let mut manager = DownloadManager::new();
            let settings = Settings {
                low_disk_threshold_mb: threshold,
                ..Settings::default()
            };
            manager.enqueue("url".into(), String::new(), Preset::Best, false);
            manager.pump_with_disk(
                &settings,
                Some(std::path::Path::new("fixture-does-not-exist")),
                Instant::now(),
                |_| {
                    assert_ne!(threshold, 0);
                    None
                },
            );
            assert!(manager.queue.is_empty());
            assert!(manager.disk_recheck_after(Instant::now()).is_none());
        }
    }

    #[test]
    fn occupied_slots_do_not_busy_loop_disk_rechecks() {
        let mut manager = DownloadManager::new();
        let settings = Settings {
            low_disk_threshold_mb: 1,
            max_concurrent_downloads: 1,
            ..Settings::default()
        };
        manager.enqueue("blocked".into(), String::new(), Preset::Best, false);
        manager.enqueue("healthy".into(), String::new(), Preset::Best, false);
        let now = Instant::now();
        let mut checks = 0;
        manager.pump_with_disk(
            &settings,
            Some(std::path::Path::new("fixture-does-not-exist")),
            now,
            |_| {
                checks += 1;
                Some(if checks == 1 { 0 } else { u64::MAX })
            },
        );
        manager.pump_with_disk(
            &settings,
            Some(std::path::Path::new("fixture-does-not-exist")),
            now + Duration::from_secs(2),
            |_| panic!("no vacant slot"),
        );
        assert_eq!(
            manager.disk_recheck_after(now + Duration::from_secs(2)),
            Some(Duration::from_secs(2))
        );
    }

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
            attempts: 0,
            playlist_all: None,
            output_dir: None,
            error: None,
            tail_lines: Vec::new(),
        };
        assert!(j.status_text().contains("42%"));
        j.state = JobState::Done;
        assert_eq!(j.status_text(), "Done");
        j.state = JobState::Paused;
        assert_eq!(j.status_text(), "Paused");
    }

    #[test]
    fn pause_queued_then_resume_requeues() {
        let mut m = DownloadManager::new();
        let id = m.enqueue("https://youtu.be/x".into(), "t".into(), Preset::Best, true);
        m.pause(id);
        assert_eq!(m.active_count(), 0);
        assert_eq!(m.paused_count(), 1);
        assert_eq!(m.paused_jobs()[0].state, JobState::Paused);
        assert!(m.resume(id));
        assert_eq!(m.paused_count(), 0);
        assert_eq!(m.queued_jobs().count(), 1);
        assert_eq!(m.queued_jobs().next().unwrap().state, JobState::Queued);
        assert!(!m.resume(id));
    }

    #[test]
    fn cancel_paused_job_moves_it_to_finished() {
        let mut m = DownloadManager::new();
        let id = m.enqueue("https://youtu.be/x".into(), "t".into(), Preset::Best, true);
        m.pause(id);
        m.cancel(id);
        assert_eq!(m.paused_count(), 0);
        assert_eq!(m.finished_jobs()[0].state, JobState::Cancelled);
    }

    #[test]
    fn pause_all_and_resume_all() {
        let mut m = DownloadManager::new();
        m.enqueue("https://youtu.be/a".into(), "a".into(), Preset::Best, true);
        m.enqueue("https://youtu.be/b".into(), "b".into(), Preset::Best, true);
        m.pause_all();
        assert_eq!(m.active_count(), 0);
        assert_eq!(m.paused_count(), 2);
        m.resume_all();
        assert_eq!(m.paused_count(), 0);
        assert_eq!(m.queued_jobs().count(), 2);
    }

    #[test]
    fn move_queued_swaps_neighbours_only() {
        let mut m = DownloadManager::new();
        let a = m.enqueue("https://youtu.be/a".into(), "a".into(), Preset::Best, true);
        let b = m.enqueue("https://youtu.be/b".into(), "b".into(), Preset::Best, true);
        assert!(!m.move_queued(a, true)); // already first
        assert!(m.move_queued(b, true));
        let order: Vec<u64> = m.queued_jobs().map(|j| j.id).collect();
        assert_eq!(order, vec![b, a]);
        assert!(m.move_queued(b, false));
        let order: Vec<u64> = m.queued_jobs().map(|j| j.id).collect();
        assert_eq!(order, vec![a, b]);
        assert!(!m.move_queued(b, false)); // already last
        assert!(!m.move_queued(999, true)); // unknown id
    }

    #[test]
    fn requeue_finished_preserves_preset_and_clears_entry() {
        let mut m = DownloadManager::new();
        let id = m.enqueue("https://youtu.be/x".into(), "t".into(), Preset::P720, false);
        m.cancel(id); // queued cancel lands in finished
        assert_eq!(m.finished_jobs().len(), 1);
        let new_id = m.requeue_finished(id).expect("requeue works");
        assert_ne!(new_id, id);
        assert!(m.finished_jobs().is_empty());
        let job = m.queued_jobs().next().unwrap();
        assert_eq!(job.preset, Preset::P720);
        assert!(!job.use_archive);
        assert_eq!(job.state, JobState::Queued);
        assert_eq!(job.attempts, 0);
        assert!(m.requeue_finished(999).is_none());
    }

    #[test]
    fn remove_and_clear_finished() {
        let mut m = DownloadManager::new();
        let a = m.enqueue("https://youtu.be/a".into(), "a".into(), Preset::Best, true);
        let b = m.enqueue("https://youtu.be/b".into(), "b".into(), Preset::Best, true);
        m.cancel(a);
        m.cancel(b);
        assert_eq!(m.finished_jobs().len(), 2);
        m.remove_finished(a);
        assert_eq!(m.finished_jobs().len(), 1);
        assert_eq!(m.finished_jobs()[0].id, b);
        m.clear_finished();
        assert!(m.finished_jobs().is_empty());
    }

    #[test]
    fn cancel_all_moves_queued_and_paused_to_finished() {
        let mut m = DownloadManager::new();
        m.enqueue("https://youtu.be/a".into(), "a".into(), Preset::Best, true);
        let b = m.enqueue("https://youtu.be/b".into(), "b".into(), Preset::Best, true);
        m.pause(b);
        m.cancel_all();
        assert_eq!(m.active_count(), 0);
        assert_eq!(m.paused_count(), 0);
        assert_eq!(m.finished_jobs().len(), 2);
        assert!(m
            .finished_jobs()
            .iter()
            .all(|j| j.state == JobState::Cancelled));
    }

    #[test]
    fn display_state_tracks_only_displayed_fields() {
        // The Updated-event gate: a line that changes nothing
        // the UI shows (only the diagnostic tail grows) must not
        // count as a change; any displayed field moving must.
        let job = Job {
            id: 1,
            url: "https://youtu.be/x".into(),
            title: "t".into(),
            preset: Preset::Best,
            state: JobState::Running,
            percent: 42.0,
            size_text: Some("10MiB".into()),
            speed_text: Some("1MiB/s".into()),
            eta_text: Some("00:09".into()),
            final_file: None,
            already_downloaded: false,
            use_archive: true,
            attempts: 0,
            playlist_all: None,
            output_dir: None,
            error: None,
            tail_lines: vec!["line".into()],
        };
        let sent = DisplayState::of(&job);
        assert!(!sent.differs_from(&job));

        let mut tail_only = job.clone();
        tail_only.tail_lines.push("another line".into());
        assert!(!sent.differs_from(&tail_only));

        let mut moved = job.clone();
        moved.percent = 43.0;
        assert!(sent.differs_from(&moved));

        let mut stated = job.clone();
        stated.state = JobState::PostProcessing;
        assert!(sent.differs_from(&stated));

        let mut errored = job.clone();
        errored.error = Some("ERROR: boom".into());
        assert!(sent.differs_from(&errored));

        let mut filed = job.clone();
        filed.final_file = Some(PathBuf::from("C:\\Videos\\x.mp4"));
        assert!(sent.differs_from(&filed));
    }

    #[test]
    fn finished_paused_event_goes_to_paused_list() {
        let mut m = DownloadManager::new();
        let id = m.enqueue("https://youtu.be/x".into(), "t".into(), Preset::Best, true);
        // Simulate a worker reporting a pause finish directly.
        let mut job = m.queued_jobs().next().unwrap().clone();
        job.state = JobState::Paused;
        m.queue.clear();
        let _ = m.tx.send(DownloadEvent::Finished(job));
        let events = m.poll_events();
        assert_eq!(events.len(), 1);
        assert_eq!(m.paused_count(), 1);
        assert!(m.finished_jobs().is_empty());
        let _ = id;
    }
}
