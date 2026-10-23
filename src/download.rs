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
            let pause = Arc::new(AtomicBool::new(false));
            let tx = self.tx.clone();
            let bin = bin.to_path_buf();
            let args = build_download_args(settings, job.preset, &job.url, job.use_archive);
            let shared = Arc::new(Mutex::new(job.clone()));
            let thread_shared = shared;
            let thread_cancel = cancel.clone();
            let thread_pause = pause.clone();
            std::thread::spawn(move || {
                run_job(bin, args, thread_shared, thread_cancel, thread_pause, tx);
            });
            self.running.push(RunningJob { job, cancel, pause });
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
        Some(self.enqueue(old.url, old.title, old.preset, old.use_archive))
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

fn run_job(
    bin: PathBuf,
    args: Vec<String>,
    shared: Arc<Mutex<Job>>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
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
        if pause.load(Ordering::SeqCst) {
            // Kill the child; the `.part` file stays, so a
            // resume continues instead of restarting.
            let _ = child.kill();
            let _ = child.wait();
            let mut job = shared.lock().unwrap();
            job.state = JobState::Paused;
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
    let paused = pause.load(Ordering::SeqCst);
    match status {
        Ok(s) if s.success() && !cancelled && !paused => {
            job.state = JobState::Done;
            job.percent = 100.0;
            if job.final_file.is_none() {
                job.final_file = last_destination;
            }
        }
        Ok(_) if cancelled => {
            job.state = JobState::Cancelled;
        }
        Ok(_) if paused => {
            job.state = JobState::Paused;
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
            attempts: 0,
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
