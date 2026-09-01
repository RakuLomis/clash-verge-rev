use anyhow::{Result, bail};
use parking_lot::Mutex;
use serde::Serialize;

use crate::singleton;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CaptureLockSnapshot {
    pub locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveCapture {
    owner_kind: String,
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
                owner_kind: Some(active.owner_kind.clone()),
                job_id: Some(active.job_id.clone()),
                reason: Some(active.reason.clone()),
            },
            None => CaptureLockSnapshot::default(),
        }
    }

    pub fn ensure_unlocked(&self, action: &str) -> Result<()> {
        if let Some(active) = self.active.lock().as_ref() {
            bail!(
                "{action} is unavailable while TrafficTracer Job {} is active: {}",
                active.job_id,
                active.reason
            );
        }
        Ok(())
    }

    pub fn acquire(&self, job_id: impl Into<String>, reason: impl Into<String>) -> Result<()> {
        self.acquire_owned("job", job_id, reason)
    }

    pub fn acquire_owned(
        &self,
        owner_kind: impl Into<String>,
        job_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<()> {
        let owner_kind = owner_kind.into();
        let job_id = job_id.into();
        let reason = reason.into();
        if owner_kind.trim().is_empty() || job_id.trim().is_empty() || reason.trim().is_empty() {
            bail!("Capture lock owner_kind, job_id and reason must not be empty");
        }

        let mut active = self.active.lock();
        if let Some(existing) = active.as_ref() {
            bail!(
                "proxy controls are locked by TrafficTracer Job {}: {}",
                existing.job_id,
                existing.reason
            );
        }
        *active = Some(ActiveCapture {
            owner_kind,
            job_id,
            reason,
        });
        Ok(())
    }

    pub fn ensure_owned(&self, owner_kind: &str, owner_id: &str, action: &str) -> Result<()> {
        let active = self.active.lock();
        let Some(existing) = active.as_ref() else {
            bail!("{action} requires an active TrafficTracer {owner_kind} lock");
        };
        if existing.owner_kind != owner_kind || existing.job_id != owner_id {
            bail!("{action} is not authorized for TrafficTracer {owner_kind} {owner_id}");
        }
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

    pub fn clear_owner_kind(&self, owner_kind: &str) -> bool {
        let mut active = self.active.lock();
        if active.as_ref().is_some_and(|capture| capture.owner_kind == owner_kind) {
            return active.take().is_some();
        }
        false
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
                owner_kind: Some("job".to_owned()),
                job_id: Some("job-one".to_owned()),
                reason: Some("capture in progress".to_owned()),
            }
        );
        assert!(lock.release("job-one").unwrap());
        assert_eq!(lock.snapshot(), CaptureLockSnapshot::default());
    }

    #[test]
    fn guarded_action_is_rejected_until_release() {
        let lock = CaptureLock::new();
        assert!(lock.ensure_unlocked("changing the proxy core").is_ok());
        lock.acquire("job-one", "capture in progress").unwrap();
        let error = lock.ensure_unlocked("changing the proxy core").unwrap_err().to_string();
        assert!(error.contains("changing the proxy core"));
        assert!(error.contains("job-one"));
        assert!(error.contains("capture in progress"));
        lock.release("job-one").unwrap();
        assert!(lock.ensure_unlocked("changing the proxy core").is_ok());
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

    #[test]
    fn pipeline_owner_authorizes_only_its_internal_transitions() {
        let lock = CaptureLock::new();
        lock.acquire_owned("pipeline", "pipeline-one", "pipeline active")
            .unwrap();
        assert!(
            lock.ensure_owned("pipeline", "pipeline-one", "switching profile")
                .is_ok()
        );
        assert!(
            lock.ensure_owned("pipeline", "pipeline-two", "switching profile")
                .is_err()
        );
        assert!(lock.ensure_owned("job", "pipeline-one", "starting batch").is_err());
        assert_eq!(lock.snapshot().owner_kind.as_deref(), Some("pipeline"));
        lock.release("pipeline-one").unwrap();
    }

    #[test]
    fn worker_failure_does_not_clear_pipeline_owner() {
        let lock = CaptureLock::new();
        lock.acquire_owned("pipeline", "pipeline-one", "pipeline active")
            .unwrap();
        assert!(!lock.clear_owner_kind("job"));
        assert!(lock.snapshot().locked);
        assert!(lock.clear_owner_kind("pipeline"));
        assert!(!lock.snapshot().locked);
    }
}
