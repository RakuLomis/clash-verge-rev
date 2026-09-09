use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use tauri::AppHandle;
use tokio::sync::broadcast;

use super::{
    client::WorkerClient,
    lock::CaptureLock,
    protocol::{EmptyParams, MessageType, Notification, NotificationMethod, RequestMethod, WORKER_API_VERSION},
    worker::{WorkerEvent, WorkerProcess},
};
use crate::singleton;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const WORKER_READY_TIMEOUT: Duration = Duration::from_secs(30);
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(7);
const PROCESS_EXIT_TIMEOUT: Duration = Duration::from_secs(2);

// Diagnostic only: never use this file as ownership or completion authority.
// Lifecycle serialization makes the per-process temporary path single-writer.
async fn record_start_checkpoint(root: &Path, attempt: &str, stage: &str) {
    let root = root.to_path_buf();
    let payload = serde_json::json!({
        "schema_version": 2,
        "supervisor_pid": std::process::id(),
        "attempt_id": attempt,
        "output_root": root,
        "stage": stage,
        "updated_at": chrono::Utc::now(),
    });
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let temporary = root.join(format!(".worker-startup-{}.tmp", std::process::id()));
        let result = fs::write(&temporary, payload.to_string())
            .and_then(|_| fs::rename(&temporary, root.join(".worker-startup.json")));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        // Preserve the previous startup after restart, without unbounded history.
        // This is diagnostic data only, never a recovery ownership authority.
        let journal = root.join(".worker-lifecycle.jsonl");
        if fs::metadata(&journal).is_ok_and(|metadata| metadata.len() >= 1024 * 1024) {
            if fs::rename(&journal, root.join(".worker-lifecycle.previous.jsonl")).is_err() {
                return; // Do not grow an unbounded journal when rotation fails.
            }
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(journal) {
            let _ = writeln!(file, "{payload}");
        }
    })
    .await;
}

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
    active_job: Arc<Mutex<Option<String>>>,
    lifecycle: tokio::sync::Mutex<()>,
    workspace: tokio::sync::Mutex<()>,
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
            active_job: Arc::new(Mutex::new(None)),
            lifecycle: tokio::sync::Mutex::new(()),
            workspace: tokio::sync::Mutex::new(()),
        }
    }

    pub fn state(&self) -> WorkerManagerState {
        self.state.lock().clone()
    }

    pub fn active_job_id(&self) -> Option<String> {
        self.active_job.lock().clone()
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
        let attempt = format!("{}-{}", std::process::id(), chrono::Utc::now().timestamp_micros());
        record_start_checkpoint(session_root, &attempt, "preparing_event_bridge").await;

        let client = Arc::new(WorkerClient::new(Arc::clone(&self.process), DEFAULT_REQUEST_TIMEOUT));
        let bridge = self.process.bridge_to_tauri(app_handle.clone());
        let monitor = self.spawn_exit_monitor();
        let mut readiness = self.process.subscribe();
        record_start_checkpoint(session_root, &attempt, "spawning_worker").await;

        let process = Arc::clone(&self.process);
        let handle = app_handle.clone();
        let root = session_root.to_path_buf();
        let endpoint = controller_endpoint.to_owned();
        let secret = controller_secret.to_owned();
        let spawned = tauri::async_runtime::spawn_blocking(move || process.start(&handle, &root, &endpoint, &secret))
            .await
            .map_err(anyhow::Error::from)
            .and_then(|result| result);
        if let Err(error) = spawned {
            record_start_checkpoint(session_root, &attempt, "spawn_failed").await;
            bridge.abort();
            monitor.abort();
            self.fail_start(error.to_string());
            return Err(error);
        }

        record_start_checkpoint(session_root, &attempt, "waiting_for_ready").await;
        let recovery = match wait_for_worker_ready(&mut readiness, session_root).await {
            Ok(recovery) => recovery,
            Err(error) => {
                record_start_checkpoint(session_root, &attempt, "ready_failed").await;
                let _ = self.stop_process().await;
                bridge.abort();
                monitor.abort();
                self.fail_start(error.to_string());
                return Err(error);
            }
        };

        record_start_checkpoint(session_root, &attempt, "waiting_for_hello").await;
        if let Err(error) = client.hello().await {
            record_start_checkpoint(session_root, &attempt, "hello_failed").await;
            let _ = self.stop_process().await;
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
        record_start_checkpoint(session_root, &attempt, "handshake_complete").await;
        self.finish_start()
    }

    pub async fn ensure_session_root(
        &self,
        app_handle: &AppHandle,
        requested_root: &Path,
        controller_endpoint: &str,
        controller_secret: &str,
        pipeline_owner: Option<&str>,
    ) -> Result<PathBuf> {
        let _workspace = self.workspace.lock().await;
        match pipeline_owner {
            Some(owner) => {
                CaptureLock::global().ensure_owned("pipeline", owner, "preparing a pipeline Worker workspace")?
            }
            None => CaptureLock::global().ensure_unlocked("TrafficTracer environment diagnostics")?,
        }
        let current_root = self.session_root.lock().clone();
        if session_root_action(&self.state(), current_root.as_deref(), requested_root)? == SessionRootAction::Reuse {
            return Ok(current_root.expect("a reused Worker has a Session root"));
        }
        let requested_root = prepare_session_root(requested_root)?;
        match session_root_action(&self.state(), current_root.as_deref(), &requested_root)? {
            SessionRootAction::Reuse => return Ok(current_root.unwrap_or(requested_root)),
            SessionRootAction::Start => {
                self.start(app_handle, &requested_root, controller_endpoint, controller_secret)
                    .await?;
                return Ok(requested_root);
            }
            SessionRootAction::Switch => {}
        }

        let previous_root = current_root.expect("a ready Worker has a Session root");
        self.begin_workspace_switch()?;
        self.graceful_stop().await?;
        if let Err(error) = self
            .start(app_handle, &requested_root, controller_endpoint, controller_secret)
            .await
        {
            let rollback = self
                .start(app_handle, &previous_root, controller_endpoint, controller_secret)
                .await;
            return match rollback {
                Ok(()) => Err(anyhow::anyhow!(
                    "failed to switch TrafficTracer Session root to {}: {error}; restored {}",
                    requested_root.display(),
                    previous_root.display()
                )),
                Err(rollback_error) => Err(anyhow::anyhow!(
                    "failed to switch TrafficTracer Session root to {}: {error}; failed to restore {}: {rollback_error}",
                    requested_root.display(),
                    previous_root.display()
                )),
            };
        }
        Ok(requested_root)
    }

    pub async fn graceful_stop(&self) -> Result<bool> {
        let _lifecycle = self.lifecycle.lock().await;
        let diagnostic_root = self.session_root.lock().clone();
        let attempt = format!("stop-{}-{}", std::process::id(), chrono::Utc::now().timestamp_micros());
        if let Some(root) = &diagnostic_root {
            record_start_checkpoint(root, &attempt, "shutdown_requested").await;
        }
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
            if let Err(error) = self.stop_process().await {
                if let Some(root) = &diagnostic_root {
                    record_start_checkpoint(root, &attempt, "shutdown_failed").await;
                }
                return Err(error);
            }
        }
        self.reset_stopped();
        if let Some(root) = &diagnostic_root {
            record_start_checkpoint(root, &attempt, "exit_confirmed").await;
        }
        Ok(acknowledged && exited)
    }

    async fn stop_process(&self) -> Result<()> {
        // Subscribe before killing: dropping the child handle is not proof of
        // process exit. Never switch workspaces until the matching exit arrives.
        let mut events = self.process.subscribe();
        let Some(instance) = self.process.instance_id() else {
            return Ok(());
        };
        let process = Arc::clone(&self.process);
        tokio::time::timeout(
            PROCESS_EXIT_TIMEOUT,
            tauri::async_runtime::spawn_blocking(move || process.stop()),
        )
        .await
        .map_err(|_| anyhow::anyhow!("WORKER_STOP_TIMEOUT: workspace switch blocked for instance {instance}"))???;
        tokio::time::timeout(PROCESS_EXIT_TIMEOUT, async {
            loop {
                match events.recv().await {
                    Ok(WorkerEvent::Exited { instance_id, .. }) if instance_id == instance => return Ok(()),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(error) => return Err(anyhow::anyhow!(error)),
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("WORKER_EXIT_UNCONFIRMED: workspace switch blocked for instance {instance}"))?
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

    pub fn mark_busy(&self, job_id: &str) -> Result<()> {
        if job_id.trim().is_empty() {
            bail!("TrafficTracer Job ID must not be empty");
        }
        let mut active_job = self.active_job.lock();
        let mut state = self.state.lock();
        if *state != WorkerManagerState::Ready || active_job.is_some() {
            bail!("TrafficTracer Worker must be ready before starting a Job");
        }
        *active_job = Some(job_id.to_owned());
        *state = WorkerManagerState::Busy;
        Ok(())
    }

    pub fn mark_ready(&self, job_id: &str) -> bool {
        let mut active_job = self.active_job.lock();
        let mut state = self.state.lock();
        if *state != WorkerManagerState::Busy || active_job.as_deref() != Some(job_id) {
            return false;
        }
        *active_job = None;
        *state = WorkerManagerState::Ready;
        true
    }

    fn begin_workspace_switch(&self) -> Result<()> {
        let active_job = self.active_job.lock();
        let mut state = self.state.lock();
        if *state != WorkerManagerState::Ready || active_job.is_some() {
            bail!("SESSION_ROOT_BUSY: TrafficTracer Worker state changed before workspace switch");
        }
        *state = WorkerManagerState::Starting;
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
        *self.active_job.lock() = None;
        CaptureLock::global().clear_owner_kind("job");
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
        *self.active_job.lock() = None;
        *self.state.lock() = WorkerManagerState::Failed { message };
    }

    fn spawn_exit_monitor(&self) -> tauri::async_runtime::JoinHandle<()> {
        let mut events = self.process.subscribe();
        let state = Arc::clone(&self.state);
        let active_job = Arc::clone(&self.active_job);
        let capture_lock = CaptureLock::global();
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(WorkerEvent::Stdout { line, .. }) => {
                        if let Some(job_id) = terminal_job_id(&line) {
                            let mut active = active_job.lock();
                            if active.as_deref() == Some(job_id.as_str()) {
                                let _ = capture_lock.release(&job_id);
                                *active = None;
                                let mut state = state.lock();
                                if *state == WorkerManagerState::Busy {
                                    *state = WorkerManagerState::Ready;
                                }
                            }
                        }
                    }
                    Ok(WorkerEvent::Exited { status, .. }) => {
                        capture_lock.clear_owner_kind("job");
                        *active_job.lock() = None;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionRootAction {
    Reuse,
    Start,
    Switch,
}

fn session_root_action(
    state: &WorkerManagerState,
    current: Option<&Path>,
    requested: &Path,
) -> Result<SessionRootAction> {
    if current.is_some_and(|path| normalize_path(path) == normalize_path(requested)) {
        return Ok(SessionRootAction::Reuse);
    }
    match state {
        WorkerManagerState::Stopped | WorkerManagerState::Failed { .. } => Ok(SessionRootAction::Start),
        WorkerManagerState::Ready => Ok(SessionRootAction::Switch),
        WorkerManagerState::Busy => bail!(
            "SESSION_ROOT_BUSY: TrafficTracer is using {} while a Job is active",
            current.map_or_else(
                || "an unknown Session root".to_owned(),
                |path| path.display().to_string()
            )
        ),
        WorkerManagerState::Starting => bail!("SESSION_ROOT_STARTING: TrafficTracer Worker is starting"),
    }
}

fn prepare_session_root(requested: &Path) -> Result<PathBuf> {
    if !requested.is_absolute() {
        bail!("TrafficTracer Session root must be an absolute path");
    }
    let create_with_private_permissions = !requested.exists();
    fs::create_dir_all(requested)?;
    #[cfg(unix)]
    if create_with_private_permissions {
        fs::set_permissions(requested, fs::Permissions::from_mode(0o700))?;
    }
    let root = dunce::canonicalize(requested)?;
    if !root.is_dir() {
        bail!("TrafficTracer Session root is not a directory: {}", root.display());
    }
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let probe = root.join(format!(".traffictracer-write-probe-{}-{nonce}", std::process::id()));
    let probe_result = (|| -> Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&probe)?;
        file.write_all(b"TrafficTracer workspace probe\n")?;
        file.sync_all()?;
        Ok(())
    })();
    let cleanup_result = fs::remove_file(&probe);
    probe_result?;
    cleanup_result?;
    Ok(root)
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

fn terminal_job_id(line: &str) -> Option<String> {
    let message = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if message.get("type").and_then(serde_json::Value::as_str) != Some("notification")
        || message.get("method").and_then(serde_json::Value::as_str) != Some("job.completed")
    {
        return None;
    }
    message.get("params")?.get("job_id")?.as_str().map(str::to_owned)
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

    struct TestRoot(PathBuf);
    impl TestRoot {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "tt-cleanup-test-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn completed_pipeline(root: &Path) -> super::super::pipeline::PipelineManifest {
        use super::super::pipeline::*;
        let mut manifest = PipelineManifest::create(
            "test-owner".into(),
            root.to_owned(),
            PipelineConfigSnapshot {
                path: root.join("sites.yaml"),
                sha256: "a".repeat(64),
            },
            vec![PipelineTarget {
                index: 0,
                url: "https://example.com/".into(),
                domain: "example.com".into(),
                duration_seconds: 8,
                network: "all".into(),
                run_label: "test".into(),
                wait_load_timeout: 30,
                page_type: "page".into(),
                playback: None,
            }],
            serde_json::json!({}),
            vec![(
                PipelineCandidate {
                    profile_uid: "test-profile".into(),
                    profile_fingerprint: "b".repeat(64),
                    profile_fingerprint_kind: PIPELINE_FINGERPRINT_SEMANTIC_V2.into(),
                    recorded_at: None,
                    selection_group: "test-group".into(),
                    requested_node: "test-node".into(),
                },
                vec!["one".into(), "two".into()],
            )],
            2,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Restored,
                error: None,
            },
        )
        .unwrap();
        manifest.state = PipelineState::CompletedWithDegraded;
        manifest.stage = PipelineStage::Finished;
        manifest.runs[0].state = PipelineRunState::Completed;
        manifest.runs[1].state = PipelineRunState::Degraded;
        for (i, run) in manifest.runs.iter_mut().enumerate() {
            run.session_ids = vec![format!("saved-session-{i}")];
            run.stage = PipelineStage::Finished;
        }
        manifest.persist().unwrap();
        manifest
    }

    struct CleanupChild {
        mode: &'static str,
        events: mpsc::Sender<CommandEvent>,
        writes: Arc<AtomicUsize>,
        kills: Arc<AtomicUsize>,
    }
    impl ManagedChild for CleanupChild {
        fn pid(&self) -> u32 {
            4242
        }
        fn write(&mut self, _: &[u8]) -> AnyResult<()> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(()) // Deliberately never acknowledge shutdown.
        }
        fn kill(self: Box<Self>) -> AnyResult<()> {
            self.kills.fetch_add(1, Ordering::SeqCst);
            if self.mode == "kill_error" {
                anyhow::bail!("injected kill failure")
            }
            if self.mode == "exit" {
                self.events.try_send(CommandEvent::Terminated(
                    tauri_plugin_shell::process::TerminatedPayload {
                        code: None,
                        signal: Some(9),
                    },
                ))?;
            }
            Ok(())
        }
    }

    async fn cleanup_case(mode: &'static str, has_job: bool, expected_lock: bool) {
        use super::super::pipeline::PipelineManifest;
        let root = TestRoot::new();
        let original = completed_pipeline(&root.0);
        let artifact = root.0.join("saved-analysis.json");
        fs::write(&artifact, b"unchanged-analysis").unwrap();
        let manager = WorkerManager::new();
        let writes = Arc::new(AtomicUsize::new(0));
        let kills = Arc::new(AtomicUsize::new(0));
        let (events, receiver) = mpsc::channel(8);
        manager
            .process
            .attach(
                receiver,
                Box::new(CleanupChild {
                    mode,
                    events: events.clone(),
                    writes: writes.clone(),
                    kills: kills.clone(),
                }),
            )
            .unwrap();
        *manager.state.lock() = if mode == "idle" {
            WorkerManagerState::Ready
        } else {
            WorkerManagerState::Failed {
                message: "injected".into(),
            }
        };
        *manager.active_job.lock() = has_job.then(|| "unfinished-job".into());
        if mode == "no_response" {
            *manager.client.lock() = Some(Arc::new(WorkerClient::new(
                manager.process.clone(),
                Duration::from_secs(1),
            )));
        }
        let lock = CaptureLock::new();
        lock.acquire_owned("pipeline", "test-owner", "test capture").unwrap();
        let path = root.0.join("pipeline-manifest.json");
        let _held_lifecycle = if mode == "lifecycle_stall" {
            Some(manager.lifecycle.lock().await)
        } else {
            None
        };
        let retained = crate::cmd::traffic_tracer::finish_pipeline_worker(
            &path,
            "test-owner",
            &manager,
            &lock,
            Duration::from_millis(100),
        )
        .await;
        assert_eq!(retained, expected_lock, "mode={mode}");
        assert_eq!(lock.snapshot().locked, expected_lock);
        if expected_lock {
            assert_eq!(lock.snapshot().job_id.as_deref(), Some("test-owner"));
            assert!(lock.acquire_owned("pipeline", "new-owner", "new capture").is_err());
        }
        let saved = PipelineManifest::load(&path).unwrap();
        assert_eq!(saved.runs, original.runs);
        assert_eq!(saved.state, original.state);
        assert_eq!(saved.stage, original.stage);
        assert_eq!(fs::read(artifact).unwrap(), b"unchanged-analysis");
        assert_eq!(saved.cleanup.as_ref().unwrap().capture_lock_retained, expected_lock);
        assert_eq!(
            saved.cleanup.unwrap().state,
            if mode == "idle" { "completed" } else { "warning" }
        );
        if mode == "idle" {
            assert_eq!(kills.load(Ordering::SeqCst), 0);
        }
        if mode == "no_response" {
            assert_eq!(writes.load(Ordering::SeqCst), 1);
        }
        // Terminate only the fake event stream; no OS process was created.
        let _ = events
            .send(CommandEvent::Terminated(
                tauri_plugin_shell::process::TerminatedPayload {
                    code: Some(0),
                    signal: None,
                },
            ))
            .await;
    }

    #[tokio::test]
    async fn supervisor_cleanup_preserves_idle_worker_and_results() {
        cleanup_case("idle", false, false).await;
    }
    #[tokio::test]
    async fn supervisor_cleanup_nonresponsive_worker_retains_lock() {
        cleanup_case("no_response", true, true).await;
    }
    #[tokio::test]
    async fn supervisor_cleanup_failed_kill_without_job_id_retains_lock() {
        cleanup_case("kill_error", false, true).await;
    }
    #[tokio::test]
    async fn supervisor_cleanup_missing_exit_retains_lock() {
        cleanup_case("no_exit", true, true).await;
    }
    #[tokio::test]
    async fn supervisor_cleanup_confirmed_forced_exit_releases_lock() {
        cleanup_case("exit", true, false).await;
    }

    #[tokio::test]
    async fn supervisor_cleanup_lifecycle_stall_retains_lock_without_job_id() {
        cleanup_case("lifecycle_stall", false, true).await;
    }

    #[tokio::test]
    async fn lifecycle_journal_retains_restart_history_and_rotates() {
        let root = TestRoot::new();
        record_start_checkpoint(&root.0, "first", "waiting_for_ready").await;
        record_start_checkpoint(&root.0, "second", "handshake_complete").await;
        let journal = root.0.join(".worker-lifecycle.jsonl");
        let text = fs::read_to_string(&journal).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("first") && text.contains("second"));
        let latest: serde_json::Value =
            serde_json::from_slice(&fs::read(root.0.join(".worker-startup.json")).unwrap()).unwrap();
        assert_eq!(latest["attempt_id"], "second");
        OpenOptions::new()
            .write(true)
            .open(&journal)
            .unwrap()
            .set_len(1024 * 1024)
            .unwrap();
        record_start_checkpoint(&root.0, "third", "shutdown_requested").await;
        assert_eq!(
            fs::metadata(root.0.join(".worker-lifecycle.previous.jsonl"))
                .unwrap()
                .len(),
            1024 * 1024
        );
        assert_eq!(fs::read_to_string(&journal).unwrap().lines().count(), 1);
        assert!(fs::read_to_string(&journal).unwrap().contains("third"));
    }

    #[tokio::test]
    async fn lifecycle_failed_rotation_does_not_grow_journal() {
        let root = TestRoot::new();
        let journal = root.0.join(".worker-lifecycle.jsonl");
        fs::write(&journal, vec![b' '; 1024 * 1024]).unwrap();
        fs::create_dir(root.0.join(".worker-lifecycle.previous.jsonl")).unwrap();
        for _ in 0..3 {
            record_start_checkpoint(&root.0, "retry", "waiting_for_ready").await;
        }
        assert_eq!(fs::metadata(&journal).unwrap().len(), 1024 * 1024);
        assert!(root.0.join(".worker-startup.json").is_file());
    }

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
            "api_version": WORKER_API_VERSION,
            "type": "notification",
            "method": "worker.ready",
            "params": {
                "version": "0.1.0",
                "api_version": WORKER_API_VERSION,
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

    #[tokio::test]
    async fn workspace_stop_waits_for_the_matching_exit() {
        let manager = WorkerManager::new();
        let kills = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = mpsc::channel::<CommandEvent>(4);
        manager
            .process
            .attach(
                receiver,
                Box::new(HungChild {
                    kills: Arc::clone(&kills),
                }),
            )
            .unwrap();
        let exit = async {
            while kills.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            assert!(manager.process.is_running());
            sender
                .send(CommandEvent::Terminated(
                    tauri_plugin_shell::process::TerminatedPayload {
                        code: None,
                        signal: Some(9),
                    },
                ))
                .await
                .unwrap();
        };
        let (stopped, _) = tokio::join!(manager.stop_process(), exit);
        stopped.unwrap();
        assert!(!manager.process.is_running());
    }

    #[test]
    fn enforces_legal_state_transitions() {
        let manager = WorkerManager::new();
        assert_eq!(manager.state(), WorkerManagerState::Stopped);

        manager.begin_start().unwrap();
        assert_eq!(manager.state(), WorkerManagerState::Starting);
        manager.finish_start().unwrap();
        assert_eq!(manager.state(), WorkerManagerState::Ready);
        manager.mark_busy("job-one").unwrap();
        assert_eq!(manager.state(), WorkerManagerState::Busy);
        assert!(manager.mark_busy("job-two").is_err());
        assert!(!manager.mark_ready("stale-job"));
        assert_eq!(manager.state(), WorkerManagerState::Busy);
        assert!(manager.mark_ready("job-one"));
        assert_eq!(manager.state(), WorkerManagerState::Ready);
        assert!(manager.finish_start().is_err());
    }

    #[test]
    fn workspace_switch_reserves_ready_state_before_shutdown() {
        let manager = WorkerManager::new();
        manager.begin_start().unwrap();
        manager.finish_start().unwrap();
        manager.begin_workspace_switch().unwrap();
        assert_eq!(manager.state(), WorkerManagerState::Starting);
        assert!(manager.mark_busy("late-job").is_err());
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
        assert_eq!(
            terminal_job_id(
                r#"{"api_version":1,"type":"notification","method":"job.completed","params":{"job_id":"job-one"}}"#
            ),
            Some("job-one".to_owned())
        );
        assert!(terminal_job_id(r#"{"api_version":1,"type":"response","method":"job.completed"}"#).is_none());
        assert!(terminal_job_id("not-json").is_none());
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

    #[test]
    fn busy_workspace_switch_preserves_the_active_job_and_root() {
        let manager = WorkerManager::new();
        let current = PathBuf::from("/tmp/traffictracer-current");
        manager.begin_start().unwrap();
        *manager.session_root.lock() = Some(current.clone());
        manager.finish_start().unwrap();
        manager.mark_busy("job-in-progress").unwrap();

        assert_eq!(
            session_root_action(&manager.state(), Some(&current), &current).unwrap(),
            SessionRootAction::Reuse
        );
        let error =
            session_root_action(&manager.state(), Some(&current), Path::new("/tmp/traffictracer-next")).unwrap_err();
        assert!(error.to_string().starts_with("SESSION_ROOT_BUSY:"));
        assert_eq!(manager.state(), WorkerManagerState::Busy);
        assert_eq!(manager.session_root().unwrap(), current);
        assert_eq!(manager.active_job.lock().as_deref(), Some("job-in-progress"));
        assert!(manager.mark_ready("job-in-progress"));
        assert_eq!(manager.state(), WorkerManagerState::Ready);
    }

    #[test]
    fn stopped_worker_starts_in_the_requested_workspace() {
        assert_eq!(
            session_root_action(&WorkerManagerState::Stopped, None, Path::new("/tmp/traffictracer-next"),).unwrap(),
            SessionRootAction::Start
        );
    }

    #[test]
    fn prepares_a_writable_canonical_session_root() {
        let root = std::env::temp_dir().join(format!(
            "traffictracer-root-test-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let nested = root.join("nested");
        let prepared = prepare_session_root(&nested).unwrap();
        assert!(prepared.is_absolute());
        assert!(prepared.is_dir());
        #[cfg(unix)]
        assert_eq!(fs::metadata(&prepared).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(prepared, dunce::canonicalize(&nested).unwrap());
        assert!(fs::read_dir(&prepared).unwrap().next().is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_relative_and_file_session_roots() {
        assert!(prepare_session_root(Path::new("relative/sessions")).is_err());
        let file = std::env::temp_dir().join(format!(
            "traffictracer-root-file-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::write(&file, b"not a directory").unwrap();
        assert!(prepare_session_root(&file).is_err());
        fs::remove_file(file).unwrap();
    }
}
