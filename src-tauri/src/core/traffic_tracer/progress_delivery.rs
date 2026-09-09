//! Bounded volatile progress only. Terminal notifications remain unchanged.
use parking_lot::Mutex;
use serde_json::Value;
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

#[derive(Default, serde::Serialize)]
pub struct Counters {
    produced: u64,
    coalesced: u64,
    submitted: u64,
    acknowledged: u64,
    dispatch_failures: u64,
}
#[derive(Default)]
pub struct Delivery {
    enabled: bool,
    latest: Option<Value>,
    in_flight: Option<(u64, Instant)>,
    last_submit: Option<Instant>,
    sequence: u64,
    counters: Counters,
    consecutive_failures: u8,
}
impl Delivery {
    pub fn offer(&mut self, value: Value) {
        self.counters.produced += 1;
        if self.latest.replace(value).is_some() {
            self.counters.coalesced += 1;
        }
    }
    pub fn acknowledge(&mut self, sequence: u64) {
        if sequence == 0 {
            self.enabled = true;
            self.consecutive_failures = 0;
            return;
        }
        if self.in_flight.is_some_and(|(id, _)| id == sequence) {
            self.in_flight = None;
            self.counters.acknowledged += 1;
            self.consecutive_failures = 0;
        }
    }
    pub fn take(&mut self, locked: bool, now: Instant) -> Option<Value> {
        if !self.enabled
            || self.consecutive_failures >= 3
            || locked
            || self.in_flight.is_some()
            || self
                .last_submit
                .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_millis(200))
        {
            return None;
        }
        let mut value = self.latest.take()?;
        let object = value.as_object_mut()?;
        self.sequence += 1;
        object.insert("delivery_sequence".into(), self.sequence.into());
        self.in_flight = Some((self.sequence, now));
        self.last_submit = Some(now);
        self.counters.submitted += 1;
        Some(value)
    }
    pub fn snapshot(&self, now: Instant) -> Value {
        serde_json::json!({"counters":self.counters,"enabled":self.enabled,
            "pending":usize::from(self.latest.is_some()),"in_flight":usize::from(self.in_flight.is_some()),"consecutive_failures":self.consecutive_failures,
            "sequence":self.sequence,"in_flight_age_ms":self.in_flight.map(|(_, at)| now.saturating_duration_since(at).as_millis())})
    }
    pub fn dispatch_failed(&mut self, sequence: u64, payload: Value) {
        if self.in_flight.is_some_and(|(id, _)| id == sequence) {
            self.in_flight = None;
            self.counters.dispatch_failures += 1;
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            if self.latest.is_none() {
                self.latest = Some(payload);
            }
        }
    }
    pub fn clear_job(&mut self, job: &str) {
        if self.latest.as_ref().and_then(|v| v["job_id"].as_str()) == Some(job) {
            self.latest = None;
        }
    }
}
pub fn global() -> &'static Mutex<Delivery> {
    static DELIVERY: OnceLock<Mutex<Delivery>> = OnceLock::new();
    DELIVERY.get_or_init(|| Mutex::new(Delivery::default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_dispatch_failures_retry_and_stop_after_three() {
        let mut d = Delivery::default();
        let now = Instant::now();
        d.acknowledge(0);
        d.offer(serde_json::json!({"job_id":"a"}));
        for n in 0..3 {
            let payload = d.take(false, now + Duration::from_secs(n)).unwrap();
            let sequence = payload["delivery_sequence"].as_u64().unwrap();
            d.dispatch_failed(sequence, payload);
        }
        assert!(d.take(false, now + Duration::from_secs(100)).is_none());
        assert_eq!(d.snapshot(now)["pending"], 1);
    }
    #[test]
    fn prolonged_nonconsumption_is_bounded_and_requires_exact_ack() {
        let mut d = Delivery::default();
        let now = Instant::now();
        d.acknowledge(0);
        d.offer(serde_json::json!({"job_id":"a","progress":0}));
        assert!(d.take(false, now).is_some());
        for i in 1..10000 {
            d.offer(serde_json::json!({"job_id":"a","progress":i}));
        }
        d.acknowledge(2);
        assert!(d.take(false, now + Duration::from_secs(900)).is_none());
        assert_eq!(d.snapshot(now)["pending"], 1);
        d.acknowledge(1);
        assert_eq!(d.take(false, now + Duration::from_secs(901)).unwrap()["progress"], 9999);
    }
    #[test]
    fn lock_pauses_submission_and_terminal_clears_pending_progress() {
        let mut d = Delivery::default();
        let now = Instant::now();
        d.offer(serde_json::json!({"job_id":"a"}));
        assert!(d.take(false, now).is_none());
        d.acknowledge(0);
        assert!(d.take(true, now).is_none());
        d.clear_job("a");
        assert!(d.take(false, now).is_none());
    }
}
