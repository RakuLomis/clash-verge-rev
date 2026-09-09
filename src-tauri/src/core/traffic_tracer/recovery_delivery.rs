//! One native unlock notification in flight; acknowledgement means JS receipt,
//! not snapshot refresh completion. No timeout release or repeated emit flood.
use parking_lot::Mutex;
use std::sync::OnceLock;

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotStatus {
    Pending,
    Complete,
    Failed,
    NotDisplayed,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotReport {
    pub generation: u64,
    pub status: SnapshotStatus,
    pub participants: u32,
}

#[derive(Default, serde::Serialize)]
pub struct RecoveryDelivery {
    ready: bool,
    latest: u64,
    in_flight: Option<u64>,
    acknowledged: u64,
    dispatch_failures: u8,
    snapshot: Option<SnapshotReport>,
}
impl RecoveryDelivery {
    pub fn snapshot_report(&mut self, report: SnapshotReport) {
        if report.generation != self.latest || report.generation == 0 || report.participants > 32 {
            return;
        }
        if (report.status == SnapshotStatus::NotDisplayed) != (report.participants == 0) {
            return;
        }
        self.snapshot = Some(report);
    }
    pub fn blocks_progress(&self) -> bool {
        self.latest > 0
            && !self.snapshot.as_ref().is_some_and(|report| {
                report.generation == self.latest
                    && matches!(report.status, SnapshotStatus::Complete | SnapshotStatus::NotDisplayed)
            })
    }
    pub fn observe(&mut self, generation: u64) {
        if generation > self.latest {
            self.dispatch_failures = 0;
        }
        self.latest = self.latest.max(generation);
    }
    pub fn dispatch_failed(&mut self, generation: u64) {
        if self.in_flight == Some(generation) {
            self.in_flight = None;
            self.dispatch_failures = self.dispatch_failures.saturating_add(1);
        }
    }
    pub fn acknowledge(&mut self, generation: u64) {
        if generation == 0 {
            self.ready = true;
        } else if self.in_flight == Some(generation) {
            self.acknowledged = generation;
            self.in_flight = None;
        }
    }
    pub fn take(&mut self, unlocked: bool) -> Option<u64> {
        if !unlocked
            || !self.ready
            || self.in_flight.is_some()
            || self.latest <= self.acknowledged
            || self.dispatch_failures >= 3
        {
            return None;
        }
        self.in_flight = Some(self.latest);
        self.in_flight
    }
}
pub fn global() -> &'static Mutex<RecoveryDelivery> {
    static STATE: OnceLock<Mutex<RecoveryDelivery>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(RecoveryDelivery::default()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_is_not_snapshot_completion_and_stale_reports_do_not_release() {
        let mut s = RecoveryDelivery::default();
        s.observe(2);
        s.acknowledge(0);
        s.take(true);
        s.acknowledge(2);
        assert!(s.blocks_progress());
        s.snapshot_report(SnapshotReport {
            generation: 1,
            status: SnapshotStatus::Complete,
            participants: 2,
        });
        assert!(s.blocks_progress());
        s.snapshot_report(SnapshotReport {
            generation: 2,
            status: SnapshotStatus::Complete,
            participants: 0,
        });
        assert!(s.blocks_progress());
        s.snapshot_report(SnapshotReport {
            generation: 2,
            status: SnapshotStatus::Failed,
            participants: 2,
        });
        assert!(s.blocks_progress());
        s.snapshot_report(SnapshotReport {
            generation: 2,
            status: SnapshotStatus::Complete,
            participants: 2,
        });
        assert!(!s.blocks_progress());
        s.observe(3);
        assert!(s.blocks_progress());
        s.snapshot_report(SnapshotReport {
            generation: 3,
            status: SnapshotStatus::NotDisplayed,
            participants: 0,
        });
        assert!(!s.blocks_progress());
    }
    #[test]
    fn coalesces_unlocks_and_never_releases_on_wrong_ack() {
        let mut s = RecoveryDelivery::default();
        s.observe(1);
        assert_eq!(s.take(true), None);
        s.acknowledge(0);
        assert_eq!(s.take(false), None);
        assert_eq!(s.take(true), Some(1));
        for generation in 2..10000 {
            s.observe(generation);
            assert_eq!(s.take(true), None);
        }
        s.acknowledge(9999);
        assert_eq!(s.take(true), None);
        s.acknowledge(1);
        assert_eq!(s.take(true), Some(9999));
        s.acknowledge(9999);
        assert_eq!(s.take(true), None);
    }
}
