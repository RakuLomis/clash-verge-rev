use std::{
    fmt,
    time::{Duration, Instant},
};

use clash_verge_logging::Type;
use parking_lot::Mutex;
use serde_json::Value;
use tauri::{AppHandle, Emitter as _};

use super::protocol::{MessageType, Notification, NotificationMethod, WORKER_API_VERSION};
use crate::logging;

pub const EVENT_WORKER_READY: &str = "traffictracer://worker-ready";
pub const EVENT_WORKER_LOG: &str = "traffictracer://worker-log";
pub const EVENT_JOB_PROGRESS: &str = "traffictracer://job-progress";
pub const EVENT_JOB_STATE: &str = "traffictracer://job-state";
pub const EVENT_JOB_COMPLETED: &str = "traffictracer://job-completed";
pub const EVENT_JOB_FAILED: &str = "traffictracer://job-failed";
pub const EVENT_JOB_CANCELLED: &str = "traffictracer://job-cancelled";

const MAX_FRONTEND_PAYLOAD_BYTES: usize = 256 * 1024;
const WORKER_LOG_MIN_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq)]
pub struct FrontendWorkerEvent {
    pub name: &'static str,
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventBridgeError {
    PayloadTooLarge { actual: usize, maximum: usize },
    InvalidNotification(String),
    VersionMismatch { actual: u32, supported: u32 },
}

impl fmt::Display for EventBridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge { actual, maximum } => {
                write!(
                    formatter,
                    "Worker event payload is {actual} bytes; maximum is {maximum}"
                )
            }
            Self::InvalidNotification(message) => {
                write!(formatter, "invalid Worker notification: {message}")
            }
            Self::VersionMismatch { actual, supported } => write!(
                formatter,
                "Worker notification API version {actual} does not match {supported}"
            ),
        }
    }
}

impl std::error::Error for EventBridgeError {}

#[derive(Default)]
pub struct NotificationMapper {
    last_worker_log: Option<Instant>,
}

impl NotificationMapper {
    pub fn map_line(&mut self, line: &str, now: Instant) -> Result<Option<FrontendWorkerEvent>, EventBridgeError> {
        let oversized = line.len() > MAX_FRONTEND_PAYLOAD_BYTES;
        let envelope: Value = serde_json::from_str(line).map_err(|error| {
            if oversized {
                EventBridgeError::PayloadTooLarge {
                    actual: line.len(),
                    maximum: MAX_FRONTEND_PAYLOAD_BYTES,
                }
            } else {
                EventBridgeError::InvalidNotification(error.to_string())
            }
        })?;
        if envelope.get("type").and_then(Value::as_str) != Some("notification") {
            return Ok(None);
        }
        let is_worker_ready = envelope.get("method").and_then(Value::as_str) == Some("worker.ready");
        if oversized && !is_worker_ready {
            return Err(EventBridgeError::PayloadTooLarge {
                actual: line.len(),
                maximum: MAX_FRONTEND_PAYLOAD_BYTES,
            });
        }
        let notification: Notification<Value> = serde_json::from_value(envelope)
            .map_err(|error| EventBridgeError::InvalidNotification(error.to_string()))?;
        if notification.kind != MessageType::Notification {
            return Err(EventBridgeError::InvalidNotification(
                "message type is not notification".to_owned(),
            ));
        }
        if notification.api_version != WORKER_API_VERSION {
            return Err(EventBridgeError::VersionMismatch {
                actual: notification.api_version,
                supported: WORKER_API_VERSION,
            });
        }

        let (name, payload) = match notification.method {
            NotificationMethod::WorkerReady => (EVENT_WORKER_READY, compact_worker_ready(&notification.params)),
            NotificationMethod::WorkerLog => {
                if self
                    .last_worker_log
                    .is_some_and(|previous| now.duration_since(previous) < WORKER_LOG_MIN_INTERVAL)
                {
                    return Ok(None);
                }
                self.last_worker_log = Some(now);
                (EVENT_WORKER_LOG, notification.params)
            }
            NotificationMethod::JobProgress => (EVENT_JOB_PROGRESS, notification.params),
            NotificationMethod::JobStateChanged => (EVENT_JOB_STATE, notification.params),
            NotificationMethod::JobCompleted => (terminal_event_name(&notification.params), notification.params),
        };
        let payload_size = serde_json::to_vec(&payload)
            .map_err(|error| EventBridgeError::InvalidNotification(error.to_string()))?
            .len();
        if payload_size > MAX_FRONTEND_PAYLOAD_BYTES {
            return Err(EventBridgeError::PayloadTooLarge {
                actual: payload_size,
                maximum: MAX_FRONTEND_PAYLOAD_BYTES,
            });
        }
        Ok(Some(FrontendWorkerEvent { name, payload }))
    }
}

fn compact_worker_ready(params: &Value) -> Value {
    let recovery = params.get("recovery").unwrap_or(&Value::Null);
    serde_json::json!({
        "version": params.get("version").and_then(Value::as_str).unwrap_or_default(),
        "api_version": params.get("api_version").and_then(Value::as_u64).unwrap_or_default(),
        "output_root": params.get("output_root").and_then(Value::as_str).unwrap_or_default(),
        "recovery": {
            "status": recovery.get("status").and_then(Value::as_str).unwrap_or("degraded"),
            "recovered_session_count": recovery.get("recovered_sessions").and_then(Value::as_array).map_or(0, Vec::len),
            "terminated_pid_count": recovery.get("terminated_pids").and_then(Value::as_array).map_or(0, Vec::len),
            "skipped_pid_count": recovery.get("skipped_pids").and_then(Value::as_array).map_or(0, Vec::len),
            "error_count": recovery.get("errors").and_then(Value::as_array).map_or(0, Vec::len),
            "summary_only": true,
        }
    })
}

fn terminal_event_name(params: &Value) -> &'static str {
    match params.get("state").and_then(Value::as_str) {
        Some("failed" | "interrupted") => EVENT_JOB_FAILED,
        Some("cancelled") => EVENT_JOB_CANCELLED,
        _ => EVENT_JOB_COMPLETED,
    }
}

pub struct TauriEventBridge {
    app_handle: AppHandle,
    mapper: Mutex<NotificationMapper>,
}

impl TauriEventBridge {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            app_handle,
            mapper: Mutex::new(NotificationMapper::default()),
        }
    }

    pub fn handle_line(&self, line: &str) {
        match self.mapper.lock().map_line(line, Instant::now()) {
            Ok(Some(event)) => {
                if let Err(error) = self.app_handle.emit(event.name, event.payload) {
                    logging!(
                        warn,
                        Type::Frontend,
                        "TrafficTracer frontend event emit failed: {}",
                        error
                    );
                }
            }
            Ok(None) => {}
            Err(error) => {
                logging!(
                    warn,
                    Type::Frontend,
                    "TrafficTracer Worker notification dropped: {}",
                    error
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(method: &str, params: Value) -> String {
        serde_json::json!({
            "api_version": WORKER_API_VERSION,
            "type": "notification",
            "method": method,
            "params": params,
        })
        .to_string()
    }

    #[test]
    fn maps_progress_and_terminal_states_to_fixed_event_names() {
        let now = Instant::now();
        let mut mapper = NotificationMapper::default();

        let progress = mapper
            .map_line(&notification("job.progress", serde_json::json!({"job_id": "one"})), now)
            .unwrap()
            .unwrap();
        assert_eq!(progress.name, EVENT_JOB_PROGRESS);

        for (state, expected) in [
            ("completed", EVENT_JOB_COMPLETED),
            ("failed", EVENT_JOB_FAILED),
            ("interrupted", EVENT_JOB_FAILED),
            ("cancelled", EVENT_JOB_CANCELLED),
        ] {
            let event = mapper
                .map_line(&notification("job.completed", serde_json::json!({"state": state})), now)
                .unwrap()
                .unwrap();
            assert_eq!(event.name, expected);
        }
    }

    #[test]
    fn throttles_worker_logs_but_not_job_events() {
        let now = Instant::now();
        let mut mapper = NotificationMapper::default();
        let log = notification("worker.log", serde_json::json!({"message": "recovery"}));

        assert!(mapper.map_line(&log, now).unwrap().is_some());
        assert!(
            mapper
                .map_line(&log, now + Duration::from_millis(50))
                .unwrap()
                .is_none()
        );
        assert!(mapper.map_line(&log, now + WORKER_LOG_MIN_INTERVAL).unwrap().is_some());

        let progress = notification("job.progress", serde_json::json!({"progress": 0.5}));
        assert!(
            mapper
                .map_line(&progress, now + Duration::from_millis(51))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn ignores_worker_responses() {
        let mut mapper = NotificationMapper::default();
        let response = serde_json::json!({
            "api_version": WORKER_API_VERSION,
            "type": "response",
            "id": 1,
            "result": {}
        })
        .to_string();

        assert!(mapper.map_line(&response, Instant::now()).unwrap().is_none());
    }

    #[test]
    fn summarizes_oversized_worker_ready_notifications() {
        let mut mapper = NotificationMapper::default();
        let recovered_sessions = (0..40)
            .map(|index| format!("session-{index}-{}", "x".repeat(8 * 1024)))
            .collect::<Vec<_>>();
        let ready = notification(
            "worker.ready",
            serde_json::json!({
                "version": "1.0.14",
                "api_version": WORKER_API_VERSION,
                "output_root": "/captures",
                "recovery": {
                    "status": "ok",
                    "recovered_sessions": recovered_sessions,
                    "terminated_pids": [1, 2],
                    "skipped_pids": [3],
                    "errors": []
                }
            }),
        );
        assert!(ready.len() > MAX_FRONTEND_PAYLOAD_BYTES);

        let event = mapper.map_line(&ready, Instant::now()).unwrap().unwrap();
        assert_eq!(event.name, EVENT_WORKER_READY);
        assert_eq!(event.payload["recovery"]["recovered_session_count"], 40);
        assert_eq!(event.payload["recovery"]["terminated_pid_count"], 2);
        assert_eq!(event.payload["recovery"]["summary_only"], true);
        assert!(serde_json::to_vec(&event.payload).unwrap().len() < MAX_FRONTEND_PAYLOAD_BYTES);
    }

    #[test]
    fn rejects_oversized_or_wrong_version_notifications() {
        let mut mapper = NotificationMapper::default();
        assert!(matches!(
            mapper.map_line(&"x".repeat(MAX_FRONTEND_PAYLOAD_BYTES + 1), Instant::now()),
            Err(EventBridgeError::PayloadTooLarge { .. })
        ));

        let wrong_version = serde_json::json!({
            "api_version": WORKER_API_VERSION + 1,
            "type": "notification",
            "method": "worker.ready",
            "params": {}
        })
        .to_string();
        assert!(matches!(
            mapper.map_line(&wrong_version, Instant::now()),
            Err(EventBridgeError::VersionMismatch { .. })
        ));
    }
}
