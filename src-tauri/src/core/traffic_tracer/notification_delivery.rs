//! Non-progress notifications are journaled before broadcast, then bounded for UI.
use super::events::FrontendWorkerEvent;
use parking_lot::Mutex;
use serde_json::Value;
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::OnceLock,
    time::{Duration, Instant},
};

pub fn open_journal(root: &Path) -> std::io::Result<File> {
    let directory = root.join("diagnostics").join("worker-notifications");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(format!(
        "{}-{}.jsonl",
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
pub fn needs_journal(line: &str) -> bool {
    if !line.contains("\"notification\"") {
        return false;
    }
    serde_json::from_str::<Value>(line)
        .ok()
        .is_some_and(|v| v["type"] == "notification" && v["method"] != "job.progress")
}
pub fn append(file: &mut File, line: &str) -> std::io::Result<()> {
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()
}
#[derive(Default)]
pub struct NotificationDelivery {
    ready: bool,
    pending: VecDeque<FrontendWorkerEvent>,
    in_flight: Option<u64>,
    sequence: u64,
    last_submit: Option<Instant>,
    pub journal_failures: u64,
    received: u64,
    coalesced: u64,
    archived_only: u64,
    dispatch_failures: u8,
}
impl NotificationDelivery {
    pub fn offer(&mut self, event: FrontendWorkerEvent) {
        if matches!(
            event.name,
            super::events::EVENT_JOB_COMPLETED | super::events::EVENT_JOB_FAILED | super::events::EVENT_JOB_CANCELLED
        ) {
            self.pending.retain(|old| {
                !(old.name == super::events::EVENT_JOB_STATE
                    && old.payload.get("job_id") == event.payload.get("job_id"))
            });
        }
        self.received += 1;
        if let Some(index) = self
            .pending
            .iter()
            .position(|old| old.name == event.name && old.payload.get("job_id") == event.payload.get("job_id"))
        {
            self.pending.remove(index);
            self.coalesced += 1;
        }
        if self.pending.len() == 32 {
            self.pending.pop_front();
            self.archived_only += 1;
        }
        self.pending.push_back(event);
    }
    pub fn acknowledge(&mut self, sequence: u64) {
        if sequence == 0 {
            self.ready = true;
            self.dispatch_failures = 0;
        } else if self.in_flight == Some(sequence) {
            self.in_flight = None;
            self.dispatch_failures = 0;
        }
    }
    pub fn take(&mut self, paused: bool, now: Instant) -> Option<(u64, FrontendWorkerEvent)> {
        if paused
            || !self.ready
            || self.dispatch_failures >= 3
            || self.in_flight.is_some()
            || self
                .last_submit
                .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_millis(200))
        {
            return None;
        }
        let event = self.pending.pop_front()?;
        self.sequence += 1;
        self.in_flight = Some(self.sequence);
        self.last_submit = Some(now);
        Some((self.sequence, event))
    }
    pub fn snapshot(&self) -> Value {
        serde_json::json!({"pending":self.pending.len(),"in_flight":self.in_flight,"sequence":self.sequence,
            "received":self.received,"coalesced":self.coalesced,"archived_only":self.archived_only,"journal_failures":self.journal_failures,"dispatch_failures":self.dispatch_failures})
    }
    pub fn dispatch_failed(&mut self, sequence: u64, event: FrontendWorkerEvent) {
        if self.in_flight == Some(sequence) {
            self.in_flight = None;
            self.dispatch_failures = self.dispatch_failures.saturating_add(1);
            if self.pending.len() == 32 {
                self.pending.pop_back();
                self.archived_only += 1;
            }
            self.pending.push_front(event);
        }
    }
}
pub fn global() -> &'static Mutex<NotificationDelivery> {
    static STATE: OnceLock<Mutex<NotificationDelivery>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(NotificationDelivery::default()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_different_jobs_and_exact_receipt() {
        let mut d = NotificationDelivery::default();
        for id in 0..10000 {
            d.offer(FrontendWorkerEvent {
                name: "terminal",
                payload: serde_json::json!({"job_id":id}),
            });
        }
        assert_eq!(d.snapshot()["pending"], 32);
        let now = Instant::now();
        d.acknowledge(0);
        assert!(d.take(true, now).is_none());
        assert!(d.take(false, now).is_some());
        d.acknowledge(9);
        assert!(d.take(false, now + Duration::from_secs(100)).is_none());
        d.acknowledge(1);
        assert!(d.take(false, now + Duration::from_secs(100)).is_some());
    }
    #[test]
    fn explicit_failures_stop_after_three_attempts_until_registration() {
        let mut d = NotificationDelivery::default();
        d.offer(FrontendWorkerEvent {
            name: "terminal",
            payload: serde_json::json!({"job_id":"one"}),
        });
        d.acknowledge(0);
        let now = Instant::now();
        for attempt in 0..3 {
            let (sequence, event) = d.take(false, now + Duration::from_secs(attempt)).unwrap();
            d.dispatch_failed(sequence + 100, event.clone());
            assert!(d.take(false, now + Duration::from_secs(100)).is_none());
            d.dispatch_failed(sequence, event);
        }
        assert_eq!(d.snapshot()["dispatch_failures"], 3);
        assert_eq!(d.snapshot()["pending"], 1);
        assert!(d.take(false, now + Duration::from_secs(100)).is_none());
        d.acknowledge(0);
        assert!(d.take(false, now + Duration::from_secs(101)).is_some());
    }
    #[test]
    fn terminal_supersedes_pending_state_for_its_job_only() {
        let mut d = NotificationDelivery::default();
        for id in ["one", "two"] {
            d.offer(FrontendWorkerEvent {
                name: super::super::events::EVENT_JOB_STATE,
                payload: serde_json::json!({"job_id":id,"state":"running"}),
            });
        }
        d.offer(FrontendWorkerEvent {
            name: super::super::events::EVENT_JOB_COMPLETED,
            payload: serde_json::json!({"job_id":"one","state":"completed"}),
        });
        assert_eq!(d.pending.len(), 2);
        assert_eq!(d.pending[0].payload["job_id"], "two");
        assert_eq!(d.pending[1].name, super::super::events::EVENT_JOB_COMPLETED);
    }
    #[test]
    fn journal_retains_full_messages_without_progress_or_response_duplication() {
        let root = std::env::temp_dir().join(format!(
            "tt-journal-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_micros()
        ));
        let mut file = open_journal(&root).unwrap();
        let line = r#"{"type":"notification","method":"worker.log","params":{"message":"full detail"}}"#;
        assert!(needs_journal(line));
        assert!(!needs_journal(r#"{"type":"notification","method":"job.progress"}"#));
        assert!(!needs_journal(r#"{"type":"response","result":{}}"#));
        append(&mut file, line).unwrap();
        append(&mut file, line).unwrap();
        let path = std::fs::read_dir(root.join("diagnostics/worker-notifications"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(std::fs::read_to_string(path).unwrap(), format!("{line}\n{line}\n"));
        drop(file);
        std::fs::remove_dir_all(root).unwrap();
    }
}
