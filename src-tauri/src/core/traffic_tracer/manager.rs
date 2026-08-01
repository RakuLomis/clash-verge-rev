use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::sync::broadcast;

use super::{
    client::WorkerClient,
    protocol::{EmptyParams, MessageType, Notification, NotificationMethod, RequestMethod, WORKER_API_VERSION},
    worker::{WorkerEvent, WorkerProcess},
};
use crate::singleton;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const WORKER_READY_TIMEOUT: Duration = Duration::from_secs(30);
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(7);
const PROCESS_EXIT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WorkerManagerState {
    Stopped,
    Starting,
    Ready,
    Busy,
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerRecoveryStatus {
    Ok,
    Degraded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRecoveryReport {
    pub status: WorkerRecoveryStatus,
    pub recovered_sessions: Vec<String>,
    pub terminated_pids: Vec<u32>,
    pub skipped_pids: Vec<u32>,
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerReadyParams {
    #[serde(rename = "version")]
    _version: String,
    api_version: u32,
    output_root: String,
    recovery: WorkerRecoveryReport,
}

#[derive(Debug, Deserialize)]
struct ShutdownResult {
    shutdown: bool,
    jobs_stopped: bool,
}

pub struct WorkerManager {
    state: Arc<Mutex<WorkerManagerState>>,
    process: Arc<WorkerProcess>,
    client: Mutex<Option<Arc<WorkerClient>>>,
    bridge: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    monitor: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    session_root: Mutex<Option<PathBuf>>,
    recovery: Mutex<Option<WorkerRecoveryReport>>,
    lifecycle: tokio::sync::Mutex<()>,
}

impl WorkerManager {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(WorkerManagerState::Stopped)),
            process: Arc::new(WorkerProcess::new()),
            client: Mutex::new(None),
            bridge: Mutex::new(None),
            monitor: Mutex::new(None),
            session_root: Mutex::new(None),
            recovery: Mutex::new(None),
            lifecycle: tokio::sync::Mutex::new(()),
        }
    }

    pub fn state(&self) -> WorkerManagerState {
        self.state.lock().clone()
    }

    pub fn client(&self) -> Result<Arc<WorkerClient>> {
        if !matches!(self.state(), WorkerManagerState::Ready | WorkerManagerState::Busy) {
            bail!("TrafficTracer Worker is not ready");
        }
        self.client
            .lock()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("TrafficTracer Worker client is unavailable"))
    }

    pub async fn start(
        &self,
        app_handle: &AppHandle,
        session_root: &Path,
        controller_endpoint: &str,
        controller_secret: &str,
    ) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        if !session_root.is_absolute() {
            bail!("TrafficTracer Session root must be an absolute path");
        }
        self.begin_start()?;

        let client = Arc::new(WorkerClient::new(Arc::clone(&self.process), DEFAULT_REQUEST_TIMEOUT));
        let bridge = self.process.bridge_to_tauri(app_handle.clone());
        let monitor = self.spawn_exit_monitor();
        let mut readiness = self.process.subscribe();

        if let Err(error) = self
            .process
            .start(app_handle, session_root, controller_endpoint, controller_secret)
        {
            bridge.abort();
            monitor.abort();
            self.fail_start(error.to_string());
            return Err(error);
        }

        let recovery = match wait_for_worker_ready(&mut readiness, session_root).await {
            Ok(recovery) => recovery,
            Err(error) => {
                let _ = self.process.stop();
                bridge.abort();
                monitor.abort();
                self.fail_start(error.to_string());
                return Err(error);
            }
        };

        if let Err(error) = client.hello().await {
            let _ = self.process.stop();
            bridge.abort();
            monitor.abort();
            self.fail_start(error.to_string());
            return Err(error.into());
        }

        *self.client.lock() = Some(client);
        *self.bridge.lock() = Some(bridge);
        *self.monitor.lock() = Some(monitor);
        *self.session_root.lock() = Some(session_root.to_path_buf());
        *self.recovery.lock() = Some(recovery);
        self.finish_start()
    }

    pub async fn graceful_stop(&self) -> Result<bool> {
        let _lifecycle = self.lifecycle.lock().await;
        if !self.process.is_running() {
            self.reset_stopped();
            return Ok(true);
        }

        let client = self.client.lock().clone();
        let acknowledged = if let Some(client) = client {
            matches!(
                tokio::time::timeout(
                    GRACEFUL_SHUTDOWN_TIMEOUT,
                    client.request::<_, ShutdownResult>(RequestMethod::WorkerShutdown, EmptyParams::default()),
                )
                .await,
                Ok(Ok(ShutdownResult {
                    shutdown: true,
                    jobs_stopped: true,
                }))
            )
        } else {
            false
        };
        let exited = acknowledged && wait_for_process_exit(&self.process, PROCESS_EXIT_TIMEOUT).await;
        if self.process.is_running() {
            let _ = self.process.stop()?;
        }
        self.reset_stopped();
        Ok(acknowledged && exited)
    }

    pub fn recovery(&self) -> Option<WorkerRecoveryReport> {
        self.recovery.lock().clone()
    }

    pub fn session_root(&self) -> Result<PathBuf> {
        self.session_root
            .lock()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("TrafficTracer Session root is unavailable"))
    }

    pub fn require_session_root(&self, requested: &Path) -> Result<PathBuf> {
        let configured = self.session_root()?;
        if normalize_path(&configured) != normalize_path(requested) {
            bail!(
                "TrafficTracer Worker is using Session root '{}'; stop it before selecting '{}'",
                configured.display(),
                requested.display()
            );
        }
        Ok(configured)
    }

    pub fn mark_busy(&self) -> Result<()> {
        let mut state = self.state.lock();
        if *state != WorkerManagerState::Ready {
            bail!("TrafficTracer Worker must be ready before starting a Job");
        }
        *state = WorkerManagerState::Busy;
        Ok(())
    }

    pub fn mark_ready(&self) -> Result<()> {
        let mut state = self.state.lock();
        if *state != WorkerManagerState::Busy {
            bail!("TrafficTracer Worker is not busy");
        }
        *state = WorkerManagerState::Ready;
        Ok(())
    }

    fn reset_stopped(&self) {
        self.client.lock().take();
        if let Some(bridge) = self.bridge.lock().take() {
            bridge.abort();
        }
        if let Some(monitor) = self.monitor.lock().take() {
            monitor.abort();
        }
        *self.session_root.lock() = None;
        *self.recovery.lock() = None;
        *self.state.lock() = WorkerManagerState::Stopped;
    }

    fn begin_start(&self) -> Result<()> {
        let mut state = self.state.lock();
        match &*state {
            WorkerManagerState::Stopped | WorkerManagerState::Failed { .. } => {
                *state = WorkerManagerState::Starting;
                Ok(())
            }
            WorkerManagerState::Starting => bail!("TrafficTracer Worker start is already in progress"),
            WorkerManagerState::Ready | WorkerManagerState::Busy => {
                bail!("TrafficTracer Worker is already running")
            }
        }
    }

    fn finish_start(&self) -> Result<()> {
        let mut state = self.state.lock();
        if *state != WorkerManagerState::Starting {
            bail!("TrafficTracer Worker cannot become ready from the current state");
        }
        *state = WorkerManagerState::Ready;
        Ok(())
    }

    fn fail_start(&self, message: String) {
        *self.client.lock() = None;
        *self.session_root.lock() = None;
        *self.recovery.lock() = None;
        *self.state.lock() = WorkerManagerState::Failed { message };
    }

    fn spawn_exit_monitor(&self) -> tauri::async_runtime::JoinHandle<()> {
        let mut events = self.process.subscribe();
        let state = Arc::clone(&self.state);
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(WorkerEvent::Stdout { line, .. }) if is_terminal_job_notification(&line) => {
                        let mut state = state.lock();
                        if *state == WorkerManagerState::Busy {
                            *state = WorkerManagerState::Ready;
                        }
                    }
                    Ok(WorkerEvent::Exited { status, .. }) => {
                        let mut state = state.lock();
                        if *state != WorkerManagerState::Stopped {
                            *state = WorkerManagerState::Failed {
                                message: format!(
                                    "TrafficTracer Worker exited (code={:?}, signal={:?})",
                                    status.code, status.signal
                                ),
                            };
                        }
                        break;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }
}

async fn wait_for_worker_ready(
    events: &mut broadcast::Receiver<WorkerEvent>,
    expected_root: &Path,
) -> Result<WorkerRecoveryReport> {
    tokio::time::timeout(WORKER_READY_TIMEOUT, async {
        loop {
            match events.recv().await {
                Ok(WorkerEvent::Stdout { line, .. }) => {
                    if let Some(result) = parse_worker_ready(&line, expected_root) {
                        return result;
                    }
                }
                Ok(WorkerEvent::Exited { status, .. }) => {
                    bail!(
                        "TrafficTracer Worker exited before recovery completed (code={:?}, signal={:?})",
                        status.code,
                        status.signal
                    );
                }
                Ok(WorkerEvent::TransportError { error, .. }) => bail!(error),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    bail!("missed {count} Worker events while waiting for recovery")
                }
                Err(broadcast::error::RecvError::Closed) => {
                    bail!("TrafficTracer Worker event stream closed before recovery completed")
                }
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for TrafficTracer Worker recovery"))?
}

fn parse_worker_ready(line: &str, expected_root: &Path) -> Option<Result<WorkerRecoveryReport>> {
    let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if value.get("type").and_then(serde_json::Value::as_str) != Some("notification")
        || value.get("method").and_then(serde_json::Value::as_str) != Some("worker.ready")
    {
        return None;
    }

    Some((|| {
        let notification: Notification<WorkerReadyParams> = serde_json::from_value(value)?;
        if notification.kind != MessageType::Notification
            || notification.method != NotificationMethod::WorkerReady
            || notification.api_version != WORKER_API_VERSION
            || notification.params.api_version != WORKER_API_VERSION
        {
            bail!("TrafficTracer Worker ready notification has an incompatible protocol version");
        }
        let actual_root = Path::new(&notification.params.output_root);
        if normalize_path(actual_root) != normalize_path(expected_root) {
            bail!(
                "TrafficTracer Worker reported unexpected Session root '{}'",
                actual_root.display()
            );
        }
        Ok(notification.params.recovery)
    })())
}

async fn wait_for_process_exit(process: &WorkerProcess, timeout: Duration) -> bool {
    tokio::time::timeout(timeout, async {
        while process.is_running() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok()
}

fn normalize_path(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn is_terminal_job_notification(line: &str) -> bool {
    let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    message.get("type").and_then(serde_json::Value::as_str) == Some("notification")
        && message.get("method").and_then(serde_json::Value::as_str) == Some("job.completed")
}

singleton!(WorkerManager, TRAFFIC_TRACER_WORKER_MANAGER);

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use anyhow::Result as AnyResult;
    use tauri_plugin_shell::process::CommandEvent;
    use tokio::sync::mpsc;

    use super::*;
    use crate::core::traffic_tracer::worker::ManagedChild;

    struct HungChild {
        kills: Arc<AtomicUsize>,
    }

    impl ManagedChild for HungChild {
        fn pid(&self) -> u32 {
            42
        }

        fn write(&mut self, _bytes: &[u8]) -> AnyResult<()> {
            Ok(())
        }

        fn kill(self: Box<Self>) -> AnyResult<()> {
            self.kills.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn ready(recovery: serde_json::Value) -> String {
        serde_json::json!({
            "api_version": 1,
            "type": "notification",
            "method": "worker.ready",
            "params": {
                "version": "0.1.0",
                "api_version": 1,
                "output_root": "/tmp/traffictracer-sessions",
                "recovery": recovery
            }
        })
        .to_string()
    }

    #[test]
    fn interrupted_sessions_are_reported_before_ready() {
        let report = parse_worker_ready(
            &ready(serde_json::json!({
                "status": "ok",
                "recovered_sessions": ["123e4567-e89b-42d3-a456-426614174000"],
                "terminated_pids": [123],
                "skipped_pids": [],
                "errors": []
            })),
            Path::new("/tmp/traffictracer-sessions"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.status, WorkerRecoveryStatus::Ok);
        assert_eq!(report.recovered_sessions.len(), 1);
        assert_eq!(report.terminated_pids, vec![123]);
    }

    #[test]
    fn recovery_failures_are_degraded_not_startup_errors() {
        let report = parse_worker_ready(
            &ready(serde_json::json!({
                "status": "degraded",
                "recovered_sessions": [],
                "terminated_pids": [],
                "skipped_pids": [456],
                "errors": ["unable to restore tracing"]
            })),
            Path::new("/tmp/traffictracer-sessions"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(report.status, WorkerRecoveryStatus::Degraded);
        assert_eq!(report.errors, vec!["unable to restore tracing"]);
    }

    #[tokio::test]
    async fn hung_worker_times_out_and_can_be_force_stopped() {
        let process = WorkerProcess::new();
        let kills = Arc::new(AtomicUsize::new(0));
        let (_sender, receiver) = mpsc::channel::<CommandEvent>(4);
        process
            .attach(
                receiver,
                Box::new(HungChild {
                    kills: Arc::clone(&kills),
                }),
            )
            .unwrap();

        assert!(!wait_for_process_exit(&process, Duration::from_millis(20)).await);
        assert!(process.stop().unwrap());
        assert_eq!(kills.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn enforces_legal_state_transitions() {
        let manager = WorkerManager::new();
        assert_eq!(manager.state(), WorkerManagerState::Stopped);

        manager.begin_start().unwrap();
        assert_eq!(manager.state(), WorkerManagerState::Starting);
        manager.finish_start().unwrap();
        assert_eq!(manager.state(), WorkerManagerState::Ready);
        manager.mark_busy().unwrap();
        assert_eq!(manager.state(), WorkerManagerState::Busy);
        assert!(manager.mark_busy().is_err());
        manager.mark_ready().unwrap();
        assert_eq!(manager.state(), WorkerManagerState::Ready);
        assert!(manager.finish_start().is_err());
    }

    #[test]
    fn failed_start_can_be_retried() {
        let manager = WorkerManager::new();
        manager.begin_start().unwrap();
        manager.fail_start("incompatible Worker".to_owned());
        assert!(matches!(manager.state(), WorkerManagerState::Failed { .. }));

        manager.begin_start().unwrap();
        assert_eq!(manager.state(), WorkerManagerState::Starting);
    }

    #[test]
    fn recognizes_only_terminal_job_notifications() {
        assert!(is_terminal_job_notification(
            r#"{"api_version":1,"type":"notification","method":"job.completed","params":{}}"#
        ));
        assert!(!is_terminal_job_notification(
            r#"{"api_version":1,"type":"response","method":"job.completed"}"#
        ));
        assert!(!is_terminal_job_notification("not-json"));
    }

    #[test]
    fn concurrent_start_has_one_winner() {
        let manager = Arc::new(WorkerManager::new());
        let successes = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|scope| {
            for _ in 0..8 {
                let manager = Arc::clone(&manager);
                let successes = Arc::clone(&successes);
                scope.spawn(move || {
                    if manager.begin_start().is_ok() {
                        successes.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        });

        assert_eq!(successes.load(Ordering::SeqCst), 1);
        assert_eq!(manager.state(), WorkerManagerState::Starting);
    }
}
