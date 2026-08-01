use anyhow::{Result, bail};
use parking_lot::Mutex;
use serde::Serialize;

use crate::singleton;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CaptureLockSnapshot {
    pub locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveCapture {
    job_id: String,
    reason: String,
}

pub struct CaptureLock {
    active: Mutex<Option<ActiveCapture>>,
}

impl CaptureLock {
    fn new() -> Self {
        Self {
            active: Mutex::new(None),
        }
    }

    pub fn snapshot(&self) -> CaptureLockSnapshot {
        match self.active.lock().as_ref() {
            Some(active) => CaptureLockSnapshot {
                locked: true,
                job_id: Some(active.job_id.clone()),
                reason: Some(active.reason.clone()),
            },
            None => CaptureLockSnapshot::default(),
        }
    }

    pub fn acquire(&self, job_id: impl Into<String>, reason: impl Into<String>) -> Result<()> {
        let job_id = job_id.into();
        let reason = reason.into();
        if job_id.trim().is_empty() || reason.trim().is_empty() {
            bail!("Capture lock job_id and reason must not be empty");
        }

        let mut active = self.active.lock();
        if let Some(existing) = active.as_ref() {
            bail!(
                "proxy controls are locked by TrafficTracer Job {}: {}",
                existing.job_id,
                existing.reason
            );
        }
        *active = Some(ActiveCapture { job_id, reason });
        Ok(())
    }

    pub fn release(&self, job_id: &str) -> Result<bool> {
        let mut active = self.active.lock();
        let Some(existing) = active.as_ref() else {
            return Ok(false);
        };
        if existing.job_id != job_id {
            bail!(
                "TrafficTracer Job {} cannot release Capture lock owned by {}",
                job_id,
                existing.job_id
            );
        }
        *active = None;
        Ok(true)
    }

    pub fn clear(&self) -> bool {
        self.active.lock().take().is_some()
    }
}

singleton!(CaptureLock, TRAFFIC_TRACER_CAPTURE_LOCK);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_exposes_job_and_reason_until_release() {
        let lock = CaptureLock::new();
        lock.acquire("job-one", "capture in progress").unwrap();
        assert_eq!(
            lock.snapshot(),
            CaptureLockSnapshot {
                locked: true,
                job_id: Some("job-one".to_owned()),
                reason: Some("capture in progress".to_owned()),
            }
        );
        assert!(lock.release("job-one").unwrap());
        assert_eq!(lock.snapshot(), CaptureLockSnapshot::default());
    }

    #[test]
    fn duplicate_release_is_idempotent_and_wrong_job_is_rejected() {
        let lock = CaptureLock::new();
        lock.acquire("job-one", "capture in progress").unwrap();
        assert!(lock.release("job-two").is_err());
        assert!(lock.snapshot().locked);
        assert!(lock.release("job-one").unwrap());
        assert!(!lock.release("job-one").unwrap());
    }

    #[test]
    fn worker_crash_clear_releases_the_lock() {
        let lock = CaptureLock::new();
        lock.acquire("job-one", "capture in progress").unwrap();
        assert!(lock.clear());
        assert!(!lock.clear());
        assert!(!lock.snapshot().locked);
    }
}
