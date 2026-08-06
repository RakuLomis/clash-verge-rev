use std::{
    fs,
    net::IpAddr,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::AppHandle;

use super::{CmdResult, StringifyErr as _};
use crate::{
    config::{Config, IVerge},
    core::{
        controller, service,
        traffic_tracer::{
            lock::{CaptureLock, CaptureLockSnapshot},
            manager::{WorkerManager, WorkerManagerState, WorkerRecoveryReport, WorkerRecoveryStatus},
            protocol::{JOB_SCHEMA_VERSION, RequestMethod},
        },
    },
    feat,
};

const TRAFFIC_TRACER_CORE: &str = "verge-mihomo-tt";
const DEFAULT_CHROME_BINARY: &str = "google-chrome";
const CAPTURE_LOCK_REASON: &str = "TrafficTracer capture is active";

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnvironmentRequest {
    pub tun_interface: String,
    pub physical_interface: String,
    pub chrome_binary: String,
    pub output_root: String,
    pub min_free_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticCheck {
    pub code: String,
    pub ok: bool,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub remediation: String,
    #[serde(default)]
    pub details: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerDiagnosticReport {
    pub ok: bool,
    pub checks: Vec<DiagnosticCheck>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CompleteIntegrationStatus {
    pub current_core: String,
    pub tun_enabled: bool,
    pub service_available: bool,
    pub configured_tun_device: String,
    pub automatic_tun_device: String,
    pub capture_tun_interface: String,
    pub worker: WorkerManagerState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompleteEnvironmentLevel {
    Ready,
    Warning,
    Blocking,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CompleteEnvironmentReport {
    pub level: CompleteEnvironmentLevel,
    pub ok: bool,
    pub checks: Vec<DiagnosticCheck>,
    pub integration: CompleteIntegrationStatus,
}

#[derive(Serialize)]
struct WorkerEnvironmentParams {
    controller_endpoint: String,
    controller_secret: String,
    tun_interface: String,
    physical_interface: String,
    chrome_binary: String,
    output_root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_free_bytes: Option<u64>,
}

#[tauri::command]
pub async fn tt_get_environment(
    app_handle: AppHandle,
    request: EnvironmentRequest,
) -> CmdResult<CompleteEnvironmentReport> {
    let verge = Config::verge().await.latest_arc();
    let current_core = verge.get_valid_clash_core().to_string();
    let tun_enabled = verge.enable_tun_mode.unwrap_or(false);
    drop(verge);

    let clash = Config::clash().await.latest_arc();
    let configured_tun_device = tun_device_from_mapping(&clash.0);
    let controller_secret = clash.get_client_info().secret.unwrap_or_default();
    drop(clash);

    let capture_tun_interface = request.tun_interface.clone();
    let service_available = service::is_service_available().await.is_ok();
    let controller_endpoint = local_controller_endpoint();
    let manager = WorkerManager::global();
    let requested_root = Path::new(&request.output_root);
    if !requested_root.is_absolute() {
        return Err("output_root must be an absolute path".into());
    }
    let active_root = manager
        .ensure_session_root(&app_handle, requested_root, &controller_endpoint, &controller_secret)
        .await
        .stringify_err()?;
    let active_root_string = active_root.to_string_lossy().into_owned();
    feat::patch_verge(
        &IVerge {
            traffic_tracer_output_root: Some(active_root_string.clone().into()),
            ..IVerge::default()
        },
        false,
    )
    .await
    .stringify_err()?;
    let client = manager.client().stringify_err()?;
    let mut worker_report = client
        .request::<_, WorkerDiagnosticReport>(
            RequestMethod::EnvironmentDiagnose,
            WorkerEnvironmentParams {
                controller_endpoint,
                controller_secret,
                tun_interface: request.tun_interface,
                physical_interface: request.physical_interface,
                chrome_binary: if request.chrome_binary.trim().is_empty() {
                    DEFAULT_CHROME_BINARY.to_owned()
                } else {
                    request.chrome_binary
                },
                output_root: active_root_string,
                min_free_bytes: request.min_free_bytes,
            },
        )
        .await
        .stringify_err()?;
    if let Some(check) = recovery_diagnostic(manager.recovery()) {
        worker_report.checks.push(check);
    }

    Ok(merge_environment(
        worker_report,
        CompleteIntegrationStatus {
            current_core,
            tun_enabled,
            service_available,
            configured_tun_device,
            automatic_tun_device: automatic_tun_device().to_owned(),
            capture_tun_interface,
            worker: manager.state(),
        },
    ))
}

fn recovery_diagnostic(recovery: Option<WorkerRecoveryReport>) -> Option<DiagnosticCheck> {
    let recovery = recovery?;
    if recovery.status != WorkerRecoveryStatus::Degraded {
        return None;
    }
    Some(DiagnosticCheck {
        code: "RECOVERY_DEGRADED".to_owned(),
        ok: false,
        severity: DiagnosticSeverity::Warning,
        message: "TrafficTracer recovery completed with warnings; historical Sessions remain available.".to_owned(),
        remediation: "Review the recovery details before starting a new capture.".to_owned(),
        details: serde_json::json!({
            "recovered_sessions": recovery.recovered_sessions,
            "terminated_pids": recovery.terminated_pids,
            "skipped_pids": recovery.skipped_pids,
            "errors": recovery.errors,
        }),
    })
}

fn tun_device_from_mapping(config: &serde_yaml_ng::Mapping) -> String {
    config
        .get("tun")
        .and_then(serde_yaml_ng::Value::as_mapping)
        .and_then(|tun| tun.get("device"))
        .and_then(serde_yaml_ng::Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

fn automatic_tun_device() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "utun1024"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "Meta"
    }
}

fn local_controller_endpoint() -> String {
    let endpoint = controller::active_ipc_path();
    if endpoint.starts_with('/') {
        format!("unix://{endpoint}")
    } else if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint
    } else {
        format!("http://{endpoint}")
    }
}

fn merge_environment(
    mut worker: WorkerDiagnosticReport,
    integration: CompleteIntegrationStatus,
) -> CompleteEnvironmentReport {
    for check in &mut worker.checks {
        normalize_core_not_traffic_tracer(check);
    }

    if integration.current_core != TRAFFIC_TRACER_CORE {
        worker.checks.insert(
            0,
            DiagnosticCheck {
                code: "CORE_NOT_TRAFFIC_TRACER".to_owned(),
                ok: false,
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "The selected core '{}' is not mihomo-traffictracer.",
                    integration.current_core
                ),
                remediation: "Select verge-mihomo-tt before starting TrafficTracer.".to_owned(),
                details: serde_json::json!({"current_core": integration.current_core}),
            },
        );
    }

    if integration.tun_enabled {
        worker.checks.push(DiagnosticCheck {
            code: if integration.service_available {
                "TUN_SERVICE_READY"
            } else {
                "TUN_SERVICE_UNAVAILABLE"
            }
            .to_owned(),
            ok: integration.service_available,
            severity: DiagnosticSeverity::Error,
            message: if integration.service_available {
                "TUN mode and the Clash Verge service are available."
            } else {
                "TUN mode is enabled but the Clash Verge service is unavailable."
            }
            .to_owned(),
            remediation: if integration.service_available {
                String::new()
            } else {
                "Install or repair the Clash Verge TUN service, then retry diagnostics.".to_owned()
            },
            details: serde_json::json!({"tun_enabled": true}),
        });
    } else {
        worker.checks.push(DiagnosticCheck {
            code: "TUN_DISABLED".to_owned(),
            ok: false,
            severity: DiagnosticSeverity::Warning,
            message: "TUN mode is disabled; proxy-flow capture may be incomplete.".to_owned(),
            remediation: "Enable TUN mode before starting a full TrafficTracer capture.".to_owned(),
            details: serde_json::json!({"tun_enabled": false}),
        });
    }

    let level = if worker
        .checks
        .iter()
        .any(|check| !check.ok && check.severity == DiagnosticSeverity::Error)
    {
        CompleteEnvironmentLevel::Blocking
    } else if worker.checks.iter().any(|check| !check.ok) {
        CompleteEnvironmentLevel::Warning
    } else {
        CompleteEnvironmentLevel::Ready
    };

    CompleteEnvironmentReport {
        ok: level != CompleteEnvironmentLevel::Blocking,
        level,
        checks: worker.checks,
        integration,
    }
}

fn normalize_core_not_traffic_tracer(check: &mut DiagnosticCheck) {
    if check.code == "CORE_TRACING_UNAVAILABLE"
        && (check.message.contains("404") || check.details.to_string().contains("404"))
    {
        check.code = "CORE_NOT_TRAFFIC_TRACER".to_owned();
        check.message = "The running Mihomo core does not expose TrafficTracer capabilities.".to_owned();
        check.remediation = "Select verge-mihomo-tt and restart the proxy core before retrying.".to_owned();
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureStartRequest {
    pub url: String,
    pub domain: String,
    pub duration_seconds: u32,
    pub network: CaptureNetwork,
    pub tun_interface: String,
    pub physical_interface: String,
    pub output_root: String,
    pub chrome_binary: String,
    #[serde(default = "default_wait_load_timeout")]
    pub wait_load_timeout: u32,
    #[serde(default = "default_run_label")]
    pub run_label: String,
    #[serde(default = "default_run_label")]
    pub page_type: String,
    #[serde(default)]
    pub target_source: TargetSource,
    #[serde(default)]
    pub options: CaptureOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureNetwork {
    Tcp,
    Udp,
    All,
}

fn default_wait_load_timeout() -> u32 {
    30
}

fn default_run_label() -> String {
    "all".to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetSource {
    Manual,
    Config {
        config_path: String,
        config_sha256: String,
        target_index: usize,
    },
}

impl Default for TargetSource {
    fn default() -> Self {
        Self::Manual
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfigPreview {
    pub schema_version: u32,
    pub config_path: String,
    pub sha256: String,
    pub targets: Vec<TargetConfigEntry>,
    pub warnings: Vec<String>,
    pub suggested_output_root: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfigEntry {
    pub index: usize,
    pub domain: String,
    pub url: String,
    pub duration_seconds: u32,
    pub network: CaptureNetwork,
    pub run_label: String,
    pub wait_load_timeout: u32,
    pub page_type: String,
}

#[derive(Serialize)]
struct TargetConfigPathParams<'a> {
    path: &'a str,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CaptureOptions {
    pub capture_packets: bool,
    pub collect_cdp: bool,
    pub collect_netlog: bool,
    pub analyze_after_capture: bool,
    pub headless: bool,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            capture_packets: true,
            collect_cdp: true,
            collect_netlog: true,
            analyze_after_capture: true,
            headless: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Created,
    Preparing,
    Capturing,
    Analyzing,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl JobState {
    fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub job_id: String,
    pub kind: String,
    pub state: JobState,
    pub stage: String,
    pub progress: f64,
    pub message: String,
    pub cancel_requested: bool,
    #[serde(default)]
    pub cancel_requested_now: Option<bool>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Serialize)]
struct CaptureJobParams {
    job: CaptureJobSpec,
}

#[derive(Serialize)]
struct CaptureJobSpec {
    schema_version: u32,
    kind: &'static str,
    job_id: String,
    url: String,
    domain: String,
    duration_seconds: u32,
    network: CaptureNetwork,
    interfaces: CaptureInterfaces,
    output_root: String,
    chrome_binary: String,
    controller: CaptureController,
    options: CaptureOptions,
    wait_load_timeout: u32,
    run_label: String,
    page_type: String,
    target_source: TargetSource,
}

#[derive(Serialize)]
struct CaptureInterfaces {
    tun: String,
    physical: String,
}

#[derive(Serialize)]
struct CaptureController {
    endpoint: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    secret: String,
}

#[derive(Serialize)]
struct JobIdParams {
    job_id: String,
}

#[derive(Serialize)]
struct CancelJobParams {
    job_id: String,
    reason: String,
}

#[tauri::command]
pub async fn tt_capture_start(app_handle: AppHandle, request: CaptureStartRequest) -> CmdResult<JobSnapshot> {
    validate_capture_request(&request)?;
    validate_config_target_is_current(&request).await?;

    let environment = tt_get_environment(
        app_handle,
        EnvironmentRequest {
            tun_interface: request.tun_interface.clone(),
            physical_interface: request.physical_interface.clone(),
            chrome_binary: request.chrome_binary.clone(),
            output_root: request.output_root.clone(),
            min_free_bytes: None,
        },
    )
    .await?;
    if environment.level == CompleteEnvironmentLevel::Blocking {
        return Err("TrafficTracer environment has blocking diagnostics".into());
    }

    let manager = WorkerManager::global();
    let client = manager.client().stringify_err()?;
    let controller_secret = Config::clash()
        .await
        .latest_arc()
        .get_client_info()
        .secret
        .unwrap_or_default();
    let job_id = new_job_id()?;
    let capture_lock = CaptureLock::global();
    capture_lock
        .acquire(job_id.clone(), CAPTURE_LOCK_REASON)
        .stringify_err()?;
    if let Err(error) = manager.mark_busy(&job_id) {
        let _ = capture_lock.release(&job_id);
        return Err(error.to_string().into());
    }
    let result = client
        .request::<_, JobSnapshot>(
            RequestMethod::JobStart,
            CaptureJobParams {
                job: CaptureJobSpec {
                    schema_version: JOB_SCHEMA_VERSION,
                    kind: "capture",
                    job_id: job_id.clone(),
                    url: request.url,
                    domain: request.domain,
                    duration_seconds: request.duration_seconds,
                    network: request.network,
                    interfaces: CaptureInterfaces {
                        tun: request.tun_interface,
                        physical: request.physical_interface,
                    },
                    output_root: request.output_root,
                    chrome_binary: request.chrome_binary,
                    controller: CaptureController {
                        endpoint: local_controller_endpoint(),
                        secret: controller_secret,
                    },
                    options: request.options,
                    wait_load_timeout: request.wait_load_timeout,
                    run_label: request.run_label,
                    page_type: request.page_type,
                    target_source: request.target_source,
                },
            },
        )
        .await;

    match result {
        Ok(snapshot) => {
            if snapshot.state.terminal() {
                let _ = capture_lock.release(&job_id);
                let _ = manager.mark_ready(&job_id);
            }
            Ok(snapshot)
        }
        Err(error) => {
            let _ = capture_lock.release(&job_id);
            let _ = manager.mark_ready(&job_id);
            Err(error.to_string().into())
        }
    }
}

#[tauri::command]
pub async fn tt_target_config_load(config_path: String) -> CmdResult<TargetConfigPreview> {
    if !Path::new(&config_path).is_absolute() {
        return Err("config_path must be an absolute path".into());
    }
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(
            RequestMethod::ConfigTargetsLoad,
            TargetConfigPathParams { path: &config_path },
        )
        .await
        .stringify_err()
}

async fn validate_config_target_is_current(request: &CaptureStartRequest) -> CmdResult {
    let TargetSource::Config {
        config_path,
        config_sha256,
        target_index,
    } = &request.target_source
    else {
        return Ok(());
    };
    let preview = tt_target_config_load(config_path.clone()).await?;
    if preview.sha256 != *config_sha256 {
        return Err("target configuration changed after loading; reload it before capture".into());
    }
    let target = preview
        .targets
        .iter()
        .find(|target| target.index == *target_index)
        .ok_or_else(|| smartstring::alias::String::from("selected target no longer exists in the configuration"))?;
    if target.url != request.url
        || target.domain != request.domain
        || target.duration_seconds != request.duration_seconds
        || target.network != request.network
        || target.run_label != request.run_label
        || target.wait_load_timeout != request.wait_load_timeout
    {
        return Err("selected target no longer matches the configuration; reload it before capture".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn tt_capture_get(job_id: String) -> CmdResult<JobSnapshot> {
    validate_job_id(&job_id)?;
    let manager = WorkerManager::global();
    let snapshot = manager
        .client()
        .stringify_err()?
        .request::<_, JobSnapshot>(RequestMethod::JobStatus, JobIdParams { job_id: job_id.clone() })
        .await
        .stringify_err()?;
    if snapshot.state.terminal() {
        let _ = CaptureLock::global().release(&job_id);
        let _ = manager.mark_ready(&job_id);
    }
    Ok(snapshot)
}

#[tauri::command]
pub async fn tt_capture_cancel(job_id: String, reason: Option<String>) -> CmdResult<JobSnapshot> {
    validate_job_id(&job_id)?;
    let manager = WorkerManager::global();
    let snapshot = manager
        .client()
        .stringify_err()?
        .request::<_, JobSnapshot>(
            RequestMethod::JobCancel,
            CancelJobParams {
                job_id: job_id.clone(),
                reason: reason.unwrap_or_else(|| "Cancelled by user.".to_owned()),
            },
        )
        .await
        .stringify_err()?;
    if snapshot.state.terminal() {
        let _ = CaptureLock::global().release(&job_id);
        let _ = manager.mark_ready(&job_id);
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn tt_get_capture_lock() -> CaptureLockSnapshot {
    CaptureLock::global().snapshot()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchStartRequest {
    pub config_path: String,
    pub config_sha256: String,
    pub targets: Vec<TargetConfigEntry>,
    pub tun_interface: String,
    pub physical_interface: String,
    pub output_root: String,
    pub chrome_binary: String,
    pub options: CaptureOptions,
    pub fail_fast: bool,
}

#[derive(Serialize)]
struct BatchJobParams {
    job: BatchJobSpec,
}

#[derive(Serialize)]
struct BatchJobSpec {
    schema_version: u32,
    kind: &'static str,
    job_id: String,
    config_path: String,
    config_sha256: String,
    targets: Vec<TargetConfigEntry>,
    interfaces: CaptureInterfaces,
    output_root: String,
    chrome_binary: String,
    controller: CaptureController,
    options: CaptureOptions,
    fail_fast: bool,
}

#[derive(Serialize)]
struct BatchIdParams {
    batch_id: String,
}

#[derive(Serialize)]
struct BatchCancelParams {
    batch_id: String,
    reason: String,
}

#[tauri::command]
pub async fn tt_batch_start(app_handle: AppHandle, request: BatchStartRequest) -> CmdResult<JobSnapshot> {
    if request.targets.is_empty() {
        return Err("batch targets must not be empty".into());
    }
    if !request.options.analyze_after_capture {
        return Err("batch requires analysis after every capture".into());
    }
    let preview = tt_target_config_load(request.config_path.clone()).await?;
    validate_batch_selection(&preview, &request)?;
    let selected_indexes = request
        .targets
        .iter()
        .map(|target| target.index)
        .collect::<std::collections::HashSet<_>>();
    let ordered_targets = preview
        .targets
        .iter()
        .filter(|target| selected_indexes.contains(&target.index))
        .cloned()
        .collect::<Vec<_>>();
    debug_assert_eq!(ordered_targets.len(), request.targets.len());
    let mut request = request;
    request.targets = ordered_targets;
    let environment = tt_get_environment(
        app_handle,
        EnvironmentRequest {
            tun_interface: request.tun_interface.clone(),
            physical_interface: request.physical_interface.clone(),
            chrome_binary: request.chrome_binary.clone(),
            output_root: request.output_root.clone(),
            min_free_bytes: None,
        },
    )
    .await?;
    if environment.level == CompleteEnvironmentLevel::Blocking {
        return Err("TrafficTracer environment has blocking diagnostics".into());
    }
    let manager = WorkerManager::global();
    let client = manager.client().stringify_err()?;
    let secret = Config::clash()
        .await
        .latest_arc()
        .get_client_info()
        .secret
        .unwrap_or_default();
    let job_id = new_job_id()?;
    let lock = CaptureLock::global();
    lock.acquire(job_id.clone(), "TrafficTracer batch capture is active")
        .stringify_err()?;
    if let Err(error) = manager.mark_busy(&job_id) {
        let _ = lock.release(&job_id);
        return Err(error.to_string().into());
    }
    let result = client
        .request::<_, JobSnapshot>(
            RequestMethod::BatchStart,
            BatchJobParams {
                job: BatchJobSpec {
                    schema_version: JOB_SCHEMA_VERSION,
                    kind: "batch",
                    job_id: job_id.clone(),
                    config_path: request.config_path,
                    config_sha256: request.config_sha256,
                    targets: request.targets,
                    interfaces: CaptureInterfaces {
                        tun: request.tun_interface,
                        physical: request.physical_interface,
                    },
                    output_root: request.output_root,
                    chrome_binary: request.chrome_binary,
                    controller: CaptureController {
                        endpoint: local_controller_endpoint(),
                        secret,
                    },
                    options: request.options,
                    fail_fast: request.fail_fast,
                },
            },
        )
        .await;
    finish_batch_request(result, &job_id, manager)
}

fn finish_batch_request(
    result: Result<JobSnapshot, crate::core::traffic_tracer::client::ClientError>,
    job_id: &str,
    manager: &WorkerManager,
) -> CmdResult<JobSnapshot> {
    match result {
        Ok(snapshot) => {
            if snapshot.state.terminal() {
                let _ = CaptureLock::global().release(job_id);
                let _ = manager.mark_ready(job_id);
            }
            Ok(snapshot)
        }
        Err(error) => {
            let _ = CaptureLock::global().release(job_id);
            let _ = manager.mark_ready(job_id);
            Err(error.to_string().into())
        }
    }
}

fn validate_batch_selection(preview: &TargetConfigPreview, request: &BatchStartRequest) -> CmdResult {
    if preview.sha256 != request.config_sha256 {
        return Err("target configuration changed after loading; reload it before batch capture".into());
    }
    let mut indexes = std::collections::HashSet::new();
    for selected in &request.targets {
        if !indexes.insert(selected.index) {
            return Err("batch target indexes must be unique".into());
        }
        if !preview.targets.iter().any(|target| target == selected) {
            return Err("selected batch target no longer matches the configuration".into());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn tt_batch_status(batch_id: String) -> CmdResult<Value> {
    validate_job_id(&batch_id)?;
    let manager = WorkerManager::global();
    let value: Value = manager
        .client()
        .stringify_err()?
        .request(
            RequestMethod::BatchStatus,
            BatchIdParams {
                batch_id: batch_id.clone(),
            },
        )
        .await
        .stringify_err()?;
    let terminal = value
        .pointer("/batch/state")
        .and_then(Value::as_str)
        .is_some_and(|state| matches!(state, "completed" | "failed" | "cancelled" | "interrupted"));
    if terminal {
        let _ = CaptureLock::global().release(&batch_id);
        let _ = manager.mark_ready(&batch_id);
    }
    Ok(value)
}

#[tauri::command]
pub async fn tt_batch_list() -> CmdResult<Value> {
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(RequestMethod::BatchList, serde_json::json!({}))
        .await
        .stringify_err()
}

#[tauri::command]
pub async fn tt_batch_cancel(batch_id: String, reason: Option<String>) -> CmdResult<Value> {
    validate_job_id(&batch_id)?;
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(
            RequestMethod::BatchCancel,
            BatchCancelParams {
                batch_id,
                reason: reason.unwrap_or_else(|| "Cancelled by user.".to_owned()),
            },
        )
        .await
        .stringify_err()
}

#[tauri::command]
pub async fn tt_batch_resume(batch_id: String) -> CmdResult<JobSnapshot> {
    validate_job_id(&batch_id)?;
    let manager = WorkerManager::global();
    let lock = CaptureLock::global();
    lock.acquire(batch_id.clone(), "TrafficTracer batch capture is active")
        .stringify_err()?;
    if let Err(error) = manager.mark_busy(&batch_id) {
        let _ = lock.release(&batch_id);
        return Err(error.to_string().into());
    }
    let result = manager
        .client()
        .stringify_err()?
        .request::<_, JobSnapshot>(
            RequestMethod::BatchResume,
            BatchIdParams {
                batch_id: batch_id.clone(),
            },
        )
        .await;
    finish_batch_request(result, &batch_id, manager)
}

fn validate_capture_request(request: &CaptureStartRequest) -> CmdResult {
    if !(request.url.starts_with("http://") || request.url.starts_with("https://"))
        || request.url.chars().any(char::is_whitespace)
    {
        return Err("url must be an absolute HTTP(S) URL".into());
    }
    if !valid_domain(&request.domain) {
        return Err("domain is invalid".into());
    }
    if !(1..=86_400).contains(&request.duration_seconds) {
        return Err("duration_seconds must be between 1 and 86400".into());
    }
    if !(1..=3_600).contains(&request.wait_load_timeout) {
        return Err("wait_load_timeout must be between 1 and 3600".into());
    }
    if !valid_run_label(&request.run_label) {
        return Err("run_label contains unsafe characters".into());
    }
    if let TargetSource::Config {
        config_path,
        config_sha256,
        ..
    } = &request.target_source
    {
        if !Path::new(config_path).is_absolute() {
            return Err("target config_path must be an absolute path".into());
        }
        if config_sha256.len() != 64 || !config_sha256.chars().all(|character| character.is_ascii_hexdigit()) {
            return Err("target config_sha256 must be a SHA-256 digest".into());
        }
    }
    if request.tun_interface.trim().is_empty() || request.physical_interface.trim().is_empty() {
        return Err("capture interfaces must not be empty".into());
    }
    for (label, path) in [
        ("output_root", request.output_root.as_str()),
        ("chrome_binary", request.chrome_binary.as_str()),
    ] {
        if !std::path::Path::new(path).is_absolute() {
            return Err(format!("{label} must be an absolute path").into());
        }
    }
    Ok(())
}

fn valid_run_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.chars().enumerate().all(|(index, character)| {
            character.is_ascii_alphanumeric() || (index > 0 && matches!(character, '.' | '_' | '-'))
        })
}

fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
}

fn new_job_id() -> CmdResult<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    ))
}

fn validate_job_id(job_id: &str) -> CmdResult {
    let parts: Vec<&str> = job_id.split('-').collect();
    if parts.len() != 5
        || [8, 4, 4, 4, 12]
            .into_iter()
            .zip(&parts)
            .any(|(length, part)| part.len() != length || !part.chars().all(|value| value.is_ascii_hexdigit()))
    {
        return Err("job_id must be a UUID".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionListResult {
    pub sessions: Vec<SessionManifest>,
    pub corrupt: Vec<CorruptSession>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionScope {
    pub scope_id: String,
    pub display_name: String,
    pub directory: String,
    pub kind: String,
    #[serde(default)]
    pub created_at: Option<String>,
    pub exists: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScopedSessionListResult {
    pub scope: SessionScope,
    pub sessions: Vec<SessionManifest>,
    pub corrupt: Vec<CorruptSession>,
}

#[derive(Default, Serialize)]
struct SessionScopeResolveParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_id: Option<String>,
}

#[derive(Serialize)]
struct SessionScopeIdParams {
    scope_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CorruptSession {
    pub session_dir: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionManifest {
    pub schema_version: u32,
    pub session_id: String,
    pub job_id: String,
    pub state: JobState,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    pub session_dir: String,
    pub target: SessionTarget,
    pub component_versions: Value,
    pub artifacts: Vec<SessionArtifact>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub error: Option<SessionError>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTarget {
    pub url: String,
    pub domain: String,
    #[serde(default)]
    pub source: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionArtifact {
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub generation_id: Option<String>,
    pub path: String,
    pub media_type: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub stage: Option<String>,
}

#[derive(Serialize)]
struct SessionIdParams {
    session_id: String,
}

#[tauri::command]
pub async fn tt_session_list() -> CmdResult<SessionListResult> {
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(RequestMethod::SessionList, serde_json::json!({}))
        .await
        .stringify_err()
}

#[tauri::command]
pub async fn tt_session_scope_resolve(
    path: Option<String>,
    job_id: Option<String>,
    batch_id: Option<String>,
) -> CmdResult<Option<SessionScope>> {
    let supplied = [path.is_some(), job_id.is_some(), batch_id.is_some()]
        .into_iter()
        .filter(|value| *value)
        .count();
    if supplied != 1 {
        return Err("exactly one of path, job_id or batch_id is required".into());
    }
    if let Some(value) = job_id.as_deref() {
        validate_job_id(value)?;
    }
    if let Some(value) = batch_id.as_deref() {
        validate_job_id(value)?;
    }
    if path.as_deref().is_some_and(|value| value.trim().is_empty()) {
        return Err("path must not be empty".into());
    }
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(
            RequestMethod::SessionScopeResolve,
            SessionScopeResolveParams { path, job_id, batch_id },
        )
        .await
        .stringify_err()
}

#[tauri::command]
pub async fn tt_session_scope_list(scope_id: String) -> CmdResult<ScopedSessionListResult> {
    if scope_id.trim().is_empty() {
        return Err("scope_id must not be empty".into());
    }
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(RequestMethod::SessionScopeList, SessionScopeIdParams { scope_id })
        .await
        .stringify_err()
}

#[tauri::command]
pub async fn tt_session_get(session_id: String) -> CmdResult<SessionManifest> {
    validate_session_id(&session_id)?;
    fetch_session(&session_id).await
}

#[tauri::command]
pub async fn tt_session_open_directory(session_id: String) -> CmdResult<String> {
    validate_session_id(&session_id)?;
    let manager = WorkerManager::global();
    let manifest = fetch_session(&session_id).await?;
    let path = resolve_session_dir(&manager.session_root().stringify_err()?, &manifest)?;
    open::that_detached(path.as_os_str()).stringify_err()?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn tt_session_open_artifact(session_id: String, artifact_id: String) -> CmdResult<String> {
    validate_session_id(&session_id)?;
    if artifact_id.trim().is_empty() {
        return Err("artifact_id must not be empty".into());
    }

    let manager = WorkerManager::global();
    let manifest = fetch_session(&session_id).await?;
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.name == artifact_id || artifact.artifact_id.as_deref() == Some(artifact_id.as_str()))
        .ok_or_else(|| smartstring::alias::String::from("artifact_id does not exist in the Session manifest"))?;
    let path = resolve_artifact_path(&manager.session_root().stringify_err()?, &manifest, artifact)?;
    open::that_detached(path.as_os_str()).stringify_err()?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn tt_session_read_analysis(session_id: String, role: String) -> CmdResult<Value> {
    const MAX_ANALYSIS_JSON_BYTES: u64 = 16 * 1024 * 1024;
    validate_session_id(&session_id)?;
    let filename = match role.as_str() {
        "request_index" => "request-index-v2.json",
        "connection_index" => "connection-index-v2.json",
        "pcap_index" => "pcap-index-v1.json",
        "coverage_summary" => "summary.json",
        _ => return Err("unsupported TrafficTracer analysis role".into()),
    };
    let manager = WorkerManager::global();
    let session_root = manager.session_root().stringify_err()?;
    let manifest = fetch_session(&session_id).await?;
    let session_dir = resolve_session_dir(&session_root, &manifest)?;
    let artifact_path = manifest
        .artifacts
        .iter()
        .rev()
        .find(|artifact| artifact.role.as_deref() == Some(role.as_str()))
        .map(|artifact| resolve_artifact_path(&session_root, &manifest, artifact))
        .transpose()?
        .or_else(|| latest_generation_artifact(&session_dir, filename))
        .unwrap_or_else(|| session_dir.join("results").join(filename));
    let canonical = artifact_path.canonicalize().stringify_err()?;
    if !canonical.starts_with(&session_dir) {
        return Err("analysis artifact escapes the Session directory".into());
    }
    let metadata = canonical.metadata().stringify_err()?;
    if !metadata.is_file() || metadata.len() > MAX_ANALYSIS_JSON_BYTES {
        return Err("analysis artifact is missing or exceeds 16 MiB".into());
    }
    let content = fs::read_to_string(&canonical).stringify_err()?;
    serde_json::from_str(&content).stringify_err()
}

fn latest_generation_artifact(session_dir: &Path, filename: &str) -> Option<PathBuf> {
    let root = session_dir.join("results").join("generations");
    let mut candidates = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path().join(filename);
            let modified = path.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    candidates.pop().map(|(_, path)| path)
}

async fn fetch_session(session_id: &str) -> CmdResult<SessionManifest> {
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(
            RequestMethod::SessionGet,
            SessionIdParams {
                session_id: session_id.to_owned(),
            },
        )
        .await
        .stringify_err()
}

fn resolve_artifact_path(
    session_root: &Path,
    manifest: &SessionManifest,
    artifact: &SessionArtifact,
) -> CmdResult<PathBuf> {
    let relative = Path::new(&artifact.path);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("artifact path must be a normalized relative path".into());
    }

    let session_dir = resolve_session_dir(session_root, manifest)?;

    let target = fs::canonicalize(session_dir.join(relative)).stringify_err()?;
    if !target.starts_with(&session_dir) || !target.is_file() {
        return Err("artifact path escapes the Session directory or is not a file".into());
    }
    Ok(target)
}

fn resolve_session_dir(session_root: &Path, manifest: &SessionManifest) -> CmdResult<PathBuf> {
    let root = fs::canonicalize(session_root).stringify_err()?;
    let session_dir = fs::canonicalize(&manifest.session_dir).stringify_err()?;
    let relative = session_dir
        .strip_prefix(&root)
        .map_err(|_| "Session directory is outside the configured Session root")?;
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Session directory is outside the configured Session root".into());
    }
    let legacy = components.len() == 1
        && session_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(&format!("_{}", manifest.session_id)));
    let grouped = components.len() == 3 && components[0].as_os_str().to_str().is_some_and(is_capture_group_name);
    if !legacy && !grouped {
        return Err("Session directory does not match a supported Session layout".into());
    }
    if !session_dir.join("manifest.json").is_file() {
        return Err("Session manifest is missing".into());
    }
    Ok(session_dir)
}

fn is_capture_group_name(value: &str) -> bool {
    value.len() == 19
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 15 => byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

fn validate_session_id(session_id: &str) -> CmdResult {
    validate_job_id(session_id).map_err(|_| "session_id must be a UUID".into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnalysisOptions {
    pub split_pcaps: bool,
    pub pcap_split_mode: String,
    pub write_flow_index: bool,
    pub overwrite: bool,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            split_pcaps: true,
            pcap_split_mode: "unique_connections".to_owned(),
            write_flow_index: true,
            overwrite: false,
        }
    }
}

#[derive(Serialize)]
struct AnalysisJobParams {
    job: AnalysisJobSpec,
}

#[derive(Serialize)]
struct AnalysisJobSpec {
    schema_version: u32,
    kind: &'static str,
    job_id: String,
    session_dir: String,
    output_root: String,
    options: AnalysisOptions,
}

#[tauri::command]
pub async fn tt_analysis_start(session_id: String, options: Option<AnalysisOptions>) -> CmdResult<JobSnapshot> {
    validate_session_id(&session_id)?;
    let manager = WorkerManager::global();
    let manifest = fetch_session(&session_id).await?;
    let root = manager.session_root().stringify_err()?;
    let session_dir = resolve_session_dir(&root, &manifest)?;
    let job_id = new_job_id()?;
    let client = manager.client().stringify_err()?;
    manager.mark_busy(&job_id).stringify_err()?;

    let result = client
        .request::<_, JobSnapshot>(
            RequestMethod::AnalysisStart,
            AnalysisJobParams {
                job: AnalysisJobSpec {
                    schema_version: JOB_SCHEMA_VERSION,
                    kind: "analysis",
                    job_id: job_id.clone(),
                    session_dir: session_dir.to_string_lossy().into_owned(),
                    output_root: root.to_string_lossy().into_owned(),
                    options: options.unwrap_or_default(),
                },
            },
        )
        .await;

    match result {
        Ok(snapshot) => Ok(snapshot),
        Err(error) => {
            let _ = manager.mark_ready(&job_id);
            Err(error.to_string().into())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowNetwork {
    Tcp,
    Udp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowQueryRequest {
    pub session_id: String,
    pub network: FlowNetwork,
    pub src_ip: String,
    pub src_port: u16,
    pub dst_ip: String,
    pub dst_port: u16,
    #[serde(default)]
    pub offset: u64,
    #[serde(default = "default_flow_limit")]
    pub limit: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowQueryResult {
    pub session_id: String,
    pub offset: u64,
    pub limit: u16,
    pub total: u64,
    pub items: Vec<FlowRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowRecord {
    pub schema_version: u32,
    pub session_id: String,
    pub flow_id: String,
    pub protocol: FlowNetwork,
    pub pre_flow: NormalizedFlowTuple,
    pub post_flow: Option<NormalizedFlowTuple>,
    pub shared: bool,
    #[serde(rename = "match")]
    pub match_info: FlowMatch,
    pub request_ids: Vec<String>,
    #[serde(default)]
    pub conn_id: Option<String>,
    #[serde(default)]
    pub outer_conn_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub resource_type: Option<String>,
    #[serde(default)]
    pub relation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormalizedFlowTuple {
    pub network: FlowNetwork,
    pub src_ip: String,
    pub src_port: u16,
    pub dst_ip: String,
    pub dst_port: u16,
    #[serde(default)]
    pub dst_host: Option<String>,
    pub complete: bool,
    pub source: String,
    pub scope: String,
    pub shared: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowMatch {
    pub status: FlowMatchStatus,
    pub confidence: f64,
    pub candidate_count: u64,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowMatchStatus {
    Matched,
    Ambiguous,
    Unmatched,
    Legacy,
}

#[derive(Serialize)]
struct FlowQueryParams {
    session_id: String,
    network: FlowNetwork,
    src_ip: String,
    src_port: u16,
    dst_ip: String,
    dst_port: u16,
    offset: u64,
    limit: u16,
}

#[tauri::command]
pub async fn tt_flow_query(request: FlowQueryRequest) -> CmdResult<FlowQueryResult> {
    let params = normalize_flow_query(request)?;
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(RequestMethod::FlowQuery, params)
        .await
        .stringify_err()
}

fn normalize_flow_query(request: FlowQueryRequest) -> CmdResult<FlowQueryParams> {
    validate_session_id(&request.session_id)?;
    if request.src_port == 0 || request.dst_port == 0 {
        return Err("flow ports must be between 1 and 65535".into());
    }
    if !(1..=1000).contains(&request.limit) {
        return Err("limit must be between 1 and 1000".into());
    }
    Ok(FlowQueryParams {
        session_id: request.session_id,
        network: request.network,
        src_ip: normalize_flow_ip(&request.src_ip)?,
        src_port: request.src_port,
        dst_ip: normalize_flow_ip(&request.dst_ip)?,
        dst_port: request.dst_port,
        offset: request.offset,
        limit: request.limit,
    })
}

fn normalize_flow_ip(value: &str) -> CmdResult<String> {
    let address: IpAddr = value.parse().map_err(|_| "flow IP address is invalid")?;
    if address.is_unspecified() {
        return Err("flow IP address must not be unspecified".into());
    }
    Ok(match address {
        IpAddr::V6(address) => address.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(address)),
        address => address,
    }
    .to_string())
}

const fn default_flow_limit() -> u16 {
    100
}

#[cfg(test)]
mod flow_tests {
    use super::*;

    fn request(session_id: &str, src_ip: &str, dst_ip: &str) -> FlowQueryRequest {
        FlowQueryRequest {
            session_id: session_id.to_owned(),
            network: FlowNetwork::Tcp,
            src_ip: src_ip.to_owned(),
            src_port: 40_000,
            dst_ip: dst_ip.to_owned(),
            dst_port: 443,
            offset: 0,
            limit: 100,
        }
    }

    #[test]
    fn normalizes_ipv4_ipv6_and_ipv4_mapped_addresses() {
        let id = "123e4567-e89b-42d3-a456-426614174000";
        let ipv4 = normalize_flow_query(request(id, "192.0.2.1", "203.0.113.8")).unwrap();
        assert_eq!(ipv4.src_ip, "192.0.2.1");

        let ipv6 = normalize_flow_query(request(id, "2001:0db8::1", "2001:db8::2")).unwrap();
        assert_eq!(ipv6.src_ip, "2001:db8::1");

        let mapped = normalize_flow_query(request(id, "::ffff:192.0.2.1", "203.0.113.8")).unwrap();
        assert_eq!(mapped.src_ip, "192.0.2.1");
    }

    #[test]
    fn rejects_unspecified_ips_and_zero_ports() {
        let id = "123e4567-e89b-42d3-a456-426614174000";
        assert!(normalize_flow_query(request(id, "0.0.0.0", "203.0.113.8")).is_err());
        let mut invalid_port = request(id, "192.0.2.1", "203.0.113.8");
        invalid_port.src_port = 0;
        assert!(normalize_flow_query(invalid_port).is_err());
    }

    #[test]
    fn no_match_is_a_valid_empty_result() {
        let result: FlowQueryResult = serde_json::from_value(serde_json::json!({
            "session_id": "123e4567-e89b-42d3-a456-426614174000",
            "offset": 0,
            "limit": 100,
            "total": 0,
            "items": []
        }))
        .unwrap();
        assert_eq!(result.total, 0);
        assert!(result.items.is_empty());
    }

    #[test]
    fn queries_remain_scoped_to_the_selected_session() {
        let first = normalize_flow_query(request(
            "123e4567-e89b-42d3-a456-426614174000",
            "192.0.2.1",
            "203.0.113.8",
        ))
        .unwrap();
        let second = normalize_flow_query(request(
            "123e4567-e89b-42d3-a456-426614174001",
            "192.0.2.1",
            "203.0.113.8",
        ))
        .unwrap();
        assert_ne!(first.session_id, second.session_id);
    }

    #[test]
    fn shared_unmatched_flow_is_preserved() {
        let result: FlowQueryResult = serde_json::from_value(serde_json::json!({
            "session_id": "123e4567-e89b-42d3-a456-426614174000",
            "offset": 0,
            "limit": 100,
            "total": 1,
            "items": [{
                "schema_version": 1,
                "session_id": "123e4567-e89b-42d3-a456-426614174000",
                "flow_id": "udp:shared",
                "protocol": "udp",
                "pre_flow": {
                    "network": "udp",
                    "src_ip": "2001:db8::10",
                    "src_port": 53000,
                    "dst_ip": "2001:db8::53",
                    "dst_port": 53,
                    "complete": true,
                    "source": "mihomo",
                    "scope": "pre_proxy",
                    "shared": true
                },
                "post_flow": null,
                "shared": true,
                "match": {
                    "status": "unmatched",
                    "confidence": 0.0,
                    "candidate_count": 0,
                    "reason": "no complete post-proxy flow"
                },
                "request_ids": []
            }]
        }))
        .unwrap();
        assert!(result.items[0].shared);
        assert_eq!(result.items[0].match_info.status, FlowMatchStatus::Unmatched);
        assert!(result.items[0].post_flow.is_none());
    }

    #[test]
    fn legacy_flow_match_status_is_accepted() {
        let status: FlowMatchStatus = serde_json::from_str("\"legacy\"").unwrap();
        assert_eq!(status, FlowMatchStatus::Legacy);
    }

    #[test]
    fn analysis_defaults_produce_the_flow_index() {
        let options = AnalysisOptions::default();
        assert!(options.split_pcaps);
        assert_eq!(options.pcap_split_mode, "unique_connections");
        assert!(options.write_flow_index);
        assert!(!options.overwrite);
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use crate::core::traffic_tracer::protocol::SESSION_SCHEMA_VERSION;

    fn manifest(session_dir: &str) -> SessionManifest {
        SessionManifest {
            schema_version: SESSION_SCHEMA_VERSION,
            session_id: "123e4567-e89b-42d3-a456-426614174000".to_owned(),
            job_id: "123e4567-e89b-42d3-a456-426614174001".to_owned(),
            state: JobState::Completed,
            created_at: "2026-08-01T00:00:00Z".to_owned(),
            updated_at: "2026-08-01T00:00:01Z".to_owned(),
            started_at: Some("2026-08-01T00:00:00Z".to_owned()),
            completed_at: Some("2026-08-01T00:00:01Z".to_owned()),
            session_dir: session_dir.to_owned(),
            target: SessionTarget {
                url: "https://example.com/".to_owned(),
                domain: "example.com".to_owned(),
                source: None,
            },
            component_versions: serde_json::json!({}),
            artifacts: Vec::new(),
            warnings: Vec::new(),
            error: None,
        }
    }

    #[test]
    fn artifact_path_rejects_parent_escape_before_file_access() {
        let manifest = manifest("/tmp/sessions/20260801_123e4567-e89b-42d3-a456-426614174000");
        let artifact = SessionArtifact {
            name: "report".to_owned(),
            kind: Some("diagnostic".to_owned()),
            artifact_id: None,
            phase: None,
            role: None,
            generation_id: None,
            path: "../outside.html".to_owned(),
            media_type: "text/html".to_owned(),
            size_bytes: 1,
            sha256: None,
            created_at: None,
        };

        let error = resolve_artifact_path(Path::new("/tmp/sessions"), &manifest, &artifact).unwrap_err();
        assert!(error.contains("normalized relative path"));
    }

    #[test]
    fn corrupt_manifest_is_rejected_by_typed_decoder() {
        let result = serde_json::from_value::<SessionManifest>(serde_json::json!({
            "schema_version": 1,
            "session_id": "123e4567-e89b-42d3-a456-426614174000"
        }));
        assert!(result.is_err());
    }

    #[test]
    fn empty_session_list_is_valid() {
        let result: SessionListResult = serde_json::from_value(serde_json::json!({
            "sessions": [],
            "corrupt": []
        }))
        .unwrap();
        assert!(result.sessions.is_empty());
        assert!(result.corrupt.is_empty());
    }

    #[test]
    fn scoped_session_list_accepts_capture_group_metadata() {
        let result: ScopedSessionListResult = serde_json::from_value(serde_json::json!({
            "scope": {
                "scope_id": "20260805-110256-685",
                "display_name": "20260805-110256-685",
                "directory": "/tmp/sessions/20260805-110256-685",
                "kind": "capture_group",
                "created_at": "2026-08-05T11:02:56.685Z",
                "exists": true
            },
            "sessions": [],
            "corrupt": []
        }))
        .unwrap();
        assert_eq!(result.scope.scope_id, "20260805-110256-685");
        assert!(result.scope.exists);
    }

    #[test]
    fn capture_group_name_requires_the_exact_timestamp_shape() {
        assert!(is_capture_group_name("20260805-110256-685"));
        assert!(!is_capture_group_name("20260805T110256.685Z"));
        assert!(!is_capture_group_name(".chrome-profiles"));
        assert!(!is_capture_group_name("20260805-110256-685-extra"));
    }

    #[test]
    fn session_artifact_accepts_v1_and_v2_shapes() {
        let legacy: SessionArtifact = serde_json::from_value(serde_json::json!({
            "name": "flow-index.json",
            "kind": "derived",
            "path": "results/flow-index.json",
            "media_type": "application/json",
            "size_bytes": 42
        }))
        .unwrap();
        assert_eq!(legacy.kind.as_deref(), Some("derived"));
        assert!(legacy.role.is_none());

        let current: SessionArtifact = serde_json::from_value(serde_json::json!({
            "artifact_id": "analysis-flow-index",
            "name": "flow-index.json",
            "phase": "analysis",
            "role": "flow_index",
            "generation_id": "78fdab68-4e5d-4b67-9910-33da00a2632a",
            "path": "results/flow-index.json",
            "media_type": "application/json",
            "size_bytes": 42
        }))
        .unwrap();
        assert_eq!(current.role.as_deref(), Some("flow_index"));
        assert!(current.kind.is_none());
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    fn valid_request() -> CaptureStartRequest {
        CaptureStartRequest {
            url: "https://example.com/".to_owned(),
            domain: "example.com".to_owned(),
            duration_seconds: 15,
            network: CaptureNetwork::All,
            tun_interface: "Meta".to_owned(),
            physical_interface: "eth0".to_owned(),
            output_root: "/tmp/traffictracer".to_owned(),
            chrome_binary: "/usr/bin/google-chrome".to_owned(),
            wait_load_timeout: 30,
            run_label: "all".to_owned(),
            page_type: "capture".to_owned(),
            target_source: TargetSource::Manual,
            options: CaptureOptions::default(),
        }
    }

    #[test]
    fn capture_spec_rejects_relative_paths_and_invalid_duration() {
        let mut request = valid_request();
        request.output_root = "relative".to_owned();
        assert!(validate_capture_request(&request).is_err());

        request.output_root = "/tmp/traffictracer".to_owned();
        request.duration_seconds = 0;
        assert!(validate_capture_request(&request).is_err());
    }

    #[test]
    fn capture_spec_validates_config_target_provenance() {
        let mut request = valid_request();
        request.target_source = TargetSource::Config {
            config_path: "/tmp/sites.yaml".to_owned(),
            config_sha256: "a".repeat(64),
            target_index: 0,
        };
        validate_capture_request(&request).unwrap();

        request.target_source = TargetSource::Config {
            config_path: "sites.yaml".to_owned(),
            config_sha256: "a".repeat(64),
            target_index: 0,
        };
        assert!(validate_capture_request(&request).is_err());

        request.target_source = TargetSource::Config {
            config_path: "/tmp/sites.yaml".to_owned(),
            config_sha256: "not-a-digest".to_owned(),
            target_index: 0,
        };
        assert!(validate_capture_request(&request).is_err());
    }

    #[test]
    fn generated_job_id_is_uuid_v4() {
        let id = new_job_id().unwrap();
        validate_job_id(&id).unwrap();
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn terminal_job_states_are_idempotent_for_cancel_polling() {
        assert!(JobState::Completed.terminal());
        assert!(JobState::Failed.terminal());
        assert!(JobState::Cancelled.terminal());
        assert!(JobState::Interrupted.terminal());
        assert!(!JobState::Capturing.terminal());
    }

    fn batch_target(index: usize, domain: &str) -> TargetConfigEntry {
        TargetConfigEntry {
            index,
            domain: domain.to_owned(),
            url: format!("https://{domain}/video"),
            duration_seconds: 10,
            network: CaptureNetwork::All,
            run_label: "video".to_owned(),
            wait_load_timeout: 30,
            page_type: "video".to_owned(),
        }
    }

    fn batch_request(targets: Vec<TargetConfigEntry>) -> BatchStartRequest {
        BatchStartRequest {
            config_path: "/tmp/targets.yaml".to_owned(),
            config_sha256: "a".repeat(64),
            targets,
            tun_interface: "Meta".to_owned(),
            physical_interface: "eth0".to_owned(),
            output_root: "/tmp/sessions".to_owned(),
            chrome_binary: "/usr/bin/chromium".to_owned(),
            options: CaptureOptions::default(),
            fail_fast: true,
        }
    }

    #[test]
    fn batch_selection_rejects_changed_config_and_duplicate_indexes() {
        let targets = vec![batch_target(0, "example.com")];
        let preview = TargetConfigPreview {
            schema_version: 1,
            config_path: "/tmp/targets.yaml".to_owned(),
            sha256: "b".repeat(64),
            targets: targets.clone(),
            warnings: Vec::new(),
            suggested_output_root: None,
        };
        assert!(validate_batch_selection(&preview, &batch_request(targets.clone())).is_err());

        let preview = TargetConfigPreview {
            sha256: "a".repeat(64),
            ..preview
        };
        let request = batch_request(vec![targets[0].clone(), targets[0].clone()]);
        assert!(validate_batch_selection(&preview, &request).is_err());
    }

    #[test]
    fn batch_selection_uses_indexes_even_when_target_values_repeat() {
        let first = batch_target(3, "example.com");
        let mut second = first.clone();
        second.index = 9;
        let preview = TargetConfigPreview {
            schema_version: 1,
            config_path: "/tmp/targets.yaml".to_owned(),
            sha256: "a".repeat(64),
            targets: vec![first.clone(), second.clone()],
            warnings: Vec::new(),
            suggested_output_root: None,
        };
        validate_batch_selection(&preview, &batch_request(vec![second, first])).unwrap();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn integration(core: &str, tun_enabled: bool, service_available: bool) -> CompleteIntegrationStatus {
        CompleteIntegrationStatus {
            current_core: core.to_owned(),
            tun_enabled,
            service_available,
            configured_tun_device: String::new(),
            automatic_tun_device: "Meta".to_owned(),
            capture_tun_interface: "Meta".to_owned(),
            worker: WorkerManagerState::Ready,
        }
    }

    fn check(code: &str, ok: bool, severity: DiagnosticSeverity) -> DiagnosticCheck {
        DiagnosticCheck {
            code: code.to_owned(),
            ok,
            severity,
            message: code.to_owned(),
            remediation: String::new(),
            details: Value::Object(Default::default()),
        }
    }

    #[test]
    fn reads_configured_tun_device_without_inventing_a_default() {
        let configured: serde_yaml_ng::Mapping =
            serde_yaml_ng::from_str("tun:\n  device: \"  Mihomo-custom  \"\n").unwrap();
        assert_eq!(tun_device_from_mapping(&configured), "Mihomo-custom");

        let automatic: serde_yaml_ng::Mapping = serde_yaml_ng::from_str("tun: {}\n").unwrap();
        assert_eq!(tun_device_from_mapping(&automatic), "");
        assert!(!automatic_tun_device().is_empty());
    }

    #[test]
    fn degraded_recovery_is_a_non_blocking_warning() {
        let diagnostic = recovery_diagnostic(Some(WorkerRecoveryReport {
            status: WorkerRecoveryStatus::Degraded,
            recovered_sessions: Vec::new(),
            terminated_pids: Vec::new(),
            skipped_pids: vec![456],
            errors: vec!["unable to restore tracing".to_owned()],
        }))
        .unwrap();
        assert_eq!(diagnostic.code, "RECOVERY_DEGRADED");
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);

        let report = merge_environment(
            WorkerDiagnosticReport {
                ok: true,
                checks: vec![diagnostic],
            },
            integration(TRAFFIC_TRACER_CORE, true, true),
        );
        assert_eq!(report.level, CompleteEnvironmentLevel::Warning);
        assert!(report.ok);
    }

    #[test]
    fn ready_environment_has_no_failed_checks() {
        let report = merge_environment(
            WorkerDiagnosticReport {
                ok: true,
                checks: vec![check("CORE_READY", true, DiagnosticSeverity::Info)],
            },
            integration(TRAFFIC_TRACER_CORE, true, true),
        );

        assert_eq!(report.level, CompleteEnvironmentLevel::Ready);
        assert!(report.ok);
    }

    #[test]
    fn warning_environment_remains_usable() {
        let report = merge_environment(
            WorkerDiagnosticReport {
                ok: true,
                checks: vec![check("CORE_READY", true, DiagnosticSeverity::Info)],
            },
            integration(TRAFFIC_TRACER_CORE, false, false),
        );

        assert_eq!(report.level, CompleteEnvironmentLevel::Warning);
        assert!(report.ok);
        assert!(report.checks.iter().any(|item| item.code == "TUN_DISABLED"));
    }

    #[test]
    fn blocking_environment_normalizes_capability_404() {
        let report = merge_environment(
            WorkerDiagnosticReport {
                ok: false,
                checks: vec![DiagnosticCheck {
                    code: "CORE_TRACING_UNAVAILABLE".to_owned(),
                    ok: false,
                    severity: DiagnosticSeverity::Error,
                    message: "GET capabilities returned HTTP 404".to_owned(),
                    remediation: String::new(),
                    details: Value::Object(Default::default()),
                }],
            },
            integration(TRAFFIC_TRACER_CORE, true, true),
        );

        assert_eq!(report.level, CompleteEnvironmentLevel::Blocking);
        assert!(!report.ok);
        assert!(report.checks.iter().any(|item| item.code == "CORE_NOT_TRAFFIC_TRACER"));
    }

    #[test]
    fn standard_core_is_always_blocking() {
        let report = merge_environment(
            WorkerDiagnosticReport {
                ok: true,
                checks: vec![check("CORE_READY", true, DiagnosticSeverity::Info)],
            },
            integration("verge-mihomo", true, true),
        );

        assert_eq!(report.level, CompleteEnvironmentLevel::Blocking);
        assert_eq!(report.checks[0].code, "CORE_NOT_TRAFFIC_TRACER");
    }
}
