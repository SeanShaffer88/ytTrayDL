//! Transfer progress is derived from bytes delivered to the owned file sink.
use super::ToolFamily;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStage {
    Resolve,
    Download,
    Verify,
    Extract,
    CheckVersions,
    Activate,
    Complete,
}
impl ToolStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Resolve => "Checking Latest Official Release",
            Self::Download => "Downloading Media Tools",
            Self::Verify => "Verifying Download",
            Self::Extract => "Checking And Extracting Archive",
            Self::CheckVersions => "Checking Tool Versions",
            Self::Activate => "Activating Verified Tools",
            Self::Complete => "Tools Ready",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolProgress {
    pub operation_id: u64,
    pub family: ToolFamily,
    pub stage: ToolStage,
    pub bytes_done: u64,
    pub total_bytes: Option<u64>,
    pub bytes_per_second: Option<f64>,
    pub eta_seconds: Option<u64>,
}

/// A single replaceable update; superseded jobs cannot overwrite a retry.
#[derive(Clone, Default)]
pub struct ProgressMailbox(Arc<Mutex<MailboxState>>);
#[derive(Default)]
struct MailboxState {
    operation_id: Option<u64>,
    latest: Option<ToolProgress>,
}
impl ProgressMailbox {
    pub fn start(&self, operation_id: u64) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.operation_id = Some(operation_id);
        state.latest = None;
    }
    pub fn publish(&self, update: ToolProgress) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.operation_id == Some(update.operation_id) {
            state.latest = Some(update);
        }
    }
    pub fn take(&self) -> Option<ToolProgress> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .latest
            .take()
    }
}

pub(super) struct Reporter<'a> {
    callback: &'a mut dyn FnMut(ToolProgress),
    started: Instant,
    current: ToolProgress,
    last_emit: Duration,
    last_rate_bytes: u64,
    last_activity: Duration,
}
impl<'a> Reporter<'a> {
    pub(super) fn new(operation_id: u64, callback: &'a mut dyn FnMut(ToolProgress)) -> Self {
        Self {
            callback,
            started: Instant::now(),
            current: ToolProgress {
                operation_id,
                family: ToolFamily::YtDlp,
                stage: ToolStage::Resolve,
                bytes_done: 0,
                total_bytes: None,
                bytes_per_second: None,
                eta_seconds: None,
            },
            last_emit: Duration::ZERO,
            last_rate_bytes: 0,
            last_activity: Duration::ZERO,
        }
    }
    pub(super) fn stage(&mut self, family: ToolFamily, stage: ToolStage, total: Option<u64>) {
        self.stage_at(family, stage, total, self.started.elapsed());
    }
    fn stage_at(
        &mut self,
        family: ToolFamily,
        stage: ToolStage,
        total: Option<u64>,
        now: Duration,
    ) {
        if (self.current.family != family && stage != ToolStage::Complete)
            || stage == ToolStage::Download
            || stage == ToolStage::Resolve
        {
            self.current.bytes_done = 0;
            self.current.bytes_per_second = None;
            self.current.eta_seconds = None;
            self.last_rate_bytes = 0;
        }
        self.current.family = family;
        self.current.stage = stage;
        self.current.total_bytes = total;
        if stage != ToolStage::Download {
            self.current.bytes_per_second = None;
            self.current.eta_seconds = if stage == ToolStage::Complete {
                Some(0)
            } else {
                None
            };
        }
        self.last_emit = now;
        self.last_activity = now;
        (self.callback)(self.current.clone());
    }
    pub(super) fn bytes(&mut self, done: u64, total: Option<u64>) {
        self.bytes_at(done, total, self.started.elapsed());
    }
    fn bytes_at(&mut self, done: u64, total: Option<u64>, now: Duration) {
        if done != self.current.bytes_done {
            self.last_activity = now;
        }
        self.current.bytes_done = done;
        self.current.total_bytes = total;
        let elapsed = now.saturating_sub(self.last_emit);
        if elapsed < Duration::from_millis(200) {
            return;
        }
        let delta = done.saturating_sub(self.last_rate_bytes);
        if delta > 0 {
            self.current.bytes_per_second = Some(delta as f64 / elapsed.as_secs_f64());
        } else if now.saturating_sub(self.last_activity) >= Duration::from_secs(2) {
            self.current.bytes_per_second = Some(0.0);
        }
        self.current.eta_seconds = total
            .zip(self.current.bytes_per_second)
            .filter(|(_, speed)| *speed > 0.0)
            .map(|(total, speed)| (total.saturating_sub(done) as f64 / speed).ceil() as u64);
        self.last_emit = now;
        self.last_rate_bytes = done;
        (self.callback)(self.current.clone());
    }
    pub(super) fn complete(&mut self, family: ToolFamily, total: u64) {
        self.current.bytes_done = total;
        self.current.bytes_per_second = None;
        self.current.eta_seconds = Some(0);
        self.stage(family, ToolStage::Complete, Some(total));
        // Completion describes the committed tool, even when the previous
        // family differed during an all-tools transaction.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mailbox_retains_only_latest_and_rejects_stale_operations() {
        let mailbox = ProgressMailbox::default();
        mailbox.start(1);
        let mut update = ToolProgress {
            operation_id: 1,
            family: ToolFamily::YtDlp,
            stage: ToolStage::Download,
            bytes_done: 0,
            total_bytes: None,
            bytes_per_second: None,
            eta_seconds: None,
        };
        for bytes in 0..10_000 {
            update.bytes_done = bytes;
            mailbox.publish(update.clone());
        }
        assert_eq!(mailbox.take().unwrap().bytes_done, 9999);
        assert!(mailbox.take().is_none());
        mailbox.start(2);
        mailbox.publish(update.clone());
        assert!(mailbox.take().is_none());
        update.operation_id = 2;
        mailbox.publish(update);
        assert_eq!(mailbox.take().unwrap().operation_id, 2);
    }
    #[test]
    fn fake_clock_coalesces_transfer_and_reports_unknown_size_and_stall_honestly() {
        let mut events = Vec::new();
        {
            let mut callback = |event| events.push(event);
            let mut reporter = Reporter::new(42, &mut callback);
            reporter.stage_at(
                ToolFamily::FFmpeg,
                ToolStage::Download,
                None,
                Duration::ZERO,
            );
            for tick in 1..=100 {
                reporter.bytes_at(tick * 10, None, Duration::from_millis(tick * 10));
            }
            reporter.bytes_at(1000, None, Duration::from_secs(4));
            reporter.stage_at(
                ToolFamily::FFmpeg,
                ToolStage::Verify,
                Some(1000),
                Duration::from_secs(4),
            );
        }
        assert_eq!(events.len(), 8); // initial + five transfer updates + stall + stage
        assert!(events.iter().all(|event| event.operation_id == 42));
        assert_eq!(events[1].bytes_done, 200);
        assert!(events[1].bytes_per_second.unwrap() > 0.0);
        assert!(events[1].eta_seconds.is_none());
        assert_eq!(events[6].bytes_per_second, Some(0.0));
        assert!(events[6].eta_seconds.is_none());
        assert_eq!(events[7].stage, ToolStage::Verify);
    }
    #[test]
    fn known_size_eta_is_real_and_stage_changes_are_immediate() {
        let mut events = Vec::new();
        let mut callback = |event| events.push(event);
        let mut reporter = Reporter::new(7, &mut callback);
        reporter.stage_at(
            ToolFamily::YtDlp,
            ToolStage::Download,
            Some(1000),
            Duration::ZERO,
        );
        reporter.bytes_at(200, Some(1000), Duration::from_millis(200));
        reporter.stage_at(
            ToolFamily::YtDlp,
            ToolStage::Verify,
            Some(1000),
            Duration::from_millis(201),
        );
        assert_eq!(events.len(), 3);
        assert_eq!(events[1].eta_seconds, Some(1));
        assert_eq!(events[2].stage, ToolStage::Verify);
        assert!(events
            .iter()
            .all(|event| event.stage != ToolStage::Complete));
    }
}
