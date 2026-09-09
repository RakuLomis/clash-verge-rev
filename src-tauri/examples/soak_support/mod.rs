//! Deterministic monitor rules; no GUI, proxy or wall-clock sleeps in unit tests.
use serde_json::{Value, json};
use std::{fs, path::Path};

pub fn preserve_unsuccessful_report(status: Option<&str>) -> bool {
    matches!(status, Some("failed" | "diagnostic_only"))
}

pub fn observation_finished(deadline_reached: bool, failed: bool, diagnostic_only: bool) -> bool {
    deadline_reached || (failed && !diagnostic_only)
}

#[derive(Default)]
pub struct Health {
    event: u64,
    rendered: u64,
    event_at_ms: u64,
    render_at_ms: u64,
    pub first_failure: Option<Value>,
}
impl Health {
    pub fn observe(&mut self, elapsed_ms: u64, gap_ms: u64, event: u64, rendered: u64, errors: u64) {
        if event != self.event {
            self.event = event;
            self.event_at_ms = elapsed_ms;
        }
        if rendered != self.rendered {
            self.rendered = rendered;
            self.render_at_ms = elapsed_ms;
        }
        let reason = if errors > 0 {
            Some("JS_ERROR")
        } else if elapsed_ms <= 15_000 {
            None
        } else if gap_ms > 10_000 {
            Some("HEARTBEAT_STALLED")
        } else if elapsed_ms.saturating_sub(self.event_at_ms) > 10_000 {
            Some("EVENT_STALLED")
        } else if elapsed_ms.saturating_sub(self.render_at_ms) > 10_000 {
            Some("RENDER_STALLED")
        } else {
            None
        };
        if let Some(reason) = reason {
            self.fail(elapsed_ms, reason);
        }
    }
    pub fn fail(&mut self, elapsed_ms: u64, reason: &str) {
        if self.first_failure.is_none() {
            self.first_failure = Some(json!({"elapsed_ms":elapsed_ms,"reason":reason}));
        }
    }
}

pub fn write_report(root: &Path, report: &Value) -> std::io::Result<()> {
    let pending = root.join("result.pending.json");
    fs::write(&pending, report.to_string())?;
    fs::rename(pending, root.join("result.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_diagnostics_are_not_replaced_by_interrupted_reports() {
        assert!(preserve_unsuccessful_report(Some("diagnostic_only")));
        assert!(preserve_unsuccessful_report(Some("failed")));
        assert!(!preserve_unsuccessful_report(Some("running")));
        assert!(!preserve_unsuccessful_report(None));
    }
    #[test]
    fn diagnostic_observation_retains_failure_but_waits_for_deadline() {
        assert!(observation_finished(false, true, false));
        assert!(!observation_finished(false, true, true));
        assert!(observation_finished(true, true, true));
        assert!(observation_finished(true, false, true));
        let mut h = Health::default();
        h.observe(20_000, 12_000, 1, 1, 0);
        h.observe(21_000, 0, 2, 2, 0);
        assert_eq!(h.first_failure.unwrap()["reason"], "HEARTBEAT_STALLED");
    }
    #[test]
    fn sampling_delay_does_not_change_stall_duration() {
        let mut h = Health::default();
        h.observe(16_000, 0, 1, 1, 0);
        h.observe(25_999, 0, 1, 1, 0);
        assert!(h.first_failure.is_none());
        h.observe(26_001, 0, 1, 1, 0);
        assert_eq!(h.first_failure.unwrap()["reason"], "EVENT_STALLED");
    }
    #[test]
    fn delayed_sampling_with_progress_is_healthy() {
        let mut h = Health::default();
        for i in 1..100 {
            h.observe(i * 1800, 900, i, i, 0);
        }
        assert!(h.first_failure.is_none());
    }
    #[test]
    fn failures_are_distinct_and_first_failure_is_preserved() {
        for (gap, events, renders, errors, reason) in [
            (10_001, 2, 2, 0, "HEARTBEAT_STALLED"),
            (0, 1, 1, 0, "EVENT_STALLED"),
            (0, 2, 1, 0, "RENDER_STALLED"),
            (0, 2, 2, 1, "JS_ERROR"),
        ] {
            let mut h = Health::default();
            h.observe(1000, 0, 1, 1, 0);
            h.observe(20_000, gap, events, renders, errors);
            assert_eq!(h.first_failure.as_ref().unwrap()["reason"], reason);
            h.fail(30_000, "REPORT_WRITE_FAILED");
            assert_eq!(h.first_failure.as_ref().unwrap()["reason"], reason);
        }
    }
    #[test]
    fn no_events_after_startup_is_not_a_pass() {
        let mut h = Health::default();
        h.observe(15_000, 0, 0, 0, 0);
        assert!(h.first_failure.is_none());
        h.observe(15_001, 0, 0, 0, 0);
        assert!(h.first_failure.is_some());
    }
}
