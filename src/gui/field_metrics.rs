//! Opt-in, bounded native field measurements. Normal builds compile none of this.
use std::sync::mpsc::{sync_channel, SyncSender};
use std::time::Duration;

enum Event {
    Sample(&'static str, u64),
    Finish,
}
#[derive(Default)]
pub(super) struct Metrics {
    sender: Option<SyncSender<Event>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Metrics {
    pub fn start() -> Self {
        let Some(path) = std::env::var_os("YTTRAYDL_FIELD_UI_REPORT") else {
            return Self::default();
        };
        let Ok(file) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        else {
            return Self::default();
        };
        let (sender, receiver) = sync_channel(8192);
        let worker = std::thread::spawn(move || {
            use std::io::Write;
            let mut file = file;
            let mut samples = std::collections::BTreeMap::<&str, Vec<u64>>::new();
            let mut retained = 0usize;
            while let Ok(event) = receiver.recv() {
                match event {
                    Event::Sample(phase, value) if retained < 8192 => {
                        samples.entry(phase).or_default().push(value);
                        retained += 1;
                    }
                    Event::Sample(..) => {}
                    Event::Finish => break,
                }
            }
            let summary: std::collections::BTreeMap<_, _> = samples.iter_mut().map(|(phase, values)| {
                values.sort_unstable();
                let at = |percent: usize| values[(values.len().saturating_sub(1) * percent) / 100];
                (*phase, serde_json::json!({"count":values.len(),"p50Micros":at(50),"p95Micros":at(95),"maxMicros":values.last()}))
            }).collect();
            let _ = serde_json::to_writer_pretty(
                &mut file,
                &serde_json::json!({"schemaVersion":1,"summary":summary,"samples":samples,"boundedSamples":8192}),
            );
            let _ = file.flush();
        });
        Self {
            sender: Some(sender),
            worker: Some(worker),
        }
    }
    pub fn record(&self, phase: &'static str, elapsed: Duration) {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(Event::Sample(
                phase,
                elapsed.as_micros().min(u64::MAX as u128) as u64,
            ));
        }
    }
    pub fn finish(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Event::Finish);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Metrics {
    fn drop(&mut self) {
        self.finish();
    }
}
