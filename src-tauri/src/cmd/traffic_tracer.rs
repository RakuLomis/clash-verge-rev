use std::{
    collections::HashSet,
    fs,
    net::IpAddr,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use clash_verge_logging::{Type, logging};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Url};
use tauri_plugin_mihomo::models::Proxies;

use super::{CmdResult, StringifyErr as _};
use crate::{
    config::{Config, IProfiles, IVerge},
    core::{
        controller, handle, service,
        traffic_tracer::{
            lock::{CaptureLock, CaptureLockSnapshot},
            manager::{WorkerManager, WorkerManagerState, WorkerRecoveryReport, WorkerRecoveryStatus},
            pipeline::{
                PIPELINE_FINGERPRINT_SEMANTIC_V2, PIPELINE_MANIFEST_NAME, PIPELINE_MAX_REPETITIONS,
                PipelineApplicationIssue, PipelineCandidate, PipelineConfigSnapshot, PipelineConnectionDrain,
                PipelineError, PipelineManifest, PipelinePolicy, PipelineProfileActivation,
                PipelineProfileActivationStep, PipelineProxySnapshot, PipelineQualityPlane, PipelineRestore,
                PipelineRestoreCheck, PipelineRunEvidence, PipelineRunQuality, PipelineRunState,
                PipelineRunVerification, PipelineSelection, PipelineStage, PipelineState, PipelineTarget, RestoreState,
            },
            protocol::{JOB_SCHEMA_VERSION, RequestMethod},
            schedule::{PipelineCandidateOrderPolicy, PipelineSchedule, PipelineScheduleMode},
        },
    },
    feat,
};

const TRAFFIC_TRACER_CORE: &str = "verge-mihomo-tt";
const DEFAULT_CHROME_BINARY: &str = "google-chrome";
const CAPTURE_LOCK_REASON: &str = "TrafficTracer capture is active";

const UI_HEARTBEAT_WARN_AFTER_MS: u64 = 12_000;
const PIPELINE_CONTROLLER_TIMEOUT: Duration = Duration::from_secs(15);
const PIPELINE_PROFILE_ACTIVATION_TIMEOUT: Duration = Duration::from_secs(75);
const PIPELINE_SELECTION_TIMEOUT: Duration = Duration::from_secs(8);
const PIPELINE_END_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(3);
const PIPELINE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const PIPELINE_OWNER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3);
const PIPELINE_OWNER_HEARTBEAT_FRESH_MS: u64 = 15_000;
const PIPELINE_OWNER_FILE: &str = "pipeline-owner.json";

struct UiHeartbeatState {
    active: AtomicBool,
    last_seen_ms: AtomicU64,
    warned: AtomicBool,
    monitor_started: AtomicBool,
}

struct ActivePipeline {
    pipeline_id: String,
    manifest_path: PathBuf,
    interrupt: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PipelineOwnerRecord {
    schema_version: u32,
    pipeline_id: String,
    app_pid: u32,
    state: String,
    stage: String,
    batch_id: Option<String>,
    heartbeat_at_ms: u64,
}

#[derive(Default)]
struct PipelineRuntime {
    active: Mutex<Option<ActivePipeline>>,
}

static PIPELINE_RUNTIME: OnceLock<PipelineRuntime> = OnceLock::new();

fn pipeline_runtime() -> &'static PipelineRuntime {
    PIPELINE_RUNTIME.get_or_init(PipelineRuntime::default)
}

fn pipeline_owner_path(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(PIPELINE_OWNER_FILE)
}

fn write_pipeline_owner_record(manifest_path: &Path, pipeline_id: &str, state: &str) -> CmdResult {
    let manifest = PipelineManifest::load(manifest_path).stringify_err()?;
    let current_run = manifest.current_run_index.and_then(|index| manifest.runs.get(index));
    let stage = current_run.map_or(manifest.stage, |run| run.stage);
    let record = PipelineOwnerRecord {
        schema_version: 1,
        pipeline_id: pipeline_id.to_owned(),
        app_pid: std::process::id(),
        state: state.to_owned(),
        stage: serde_json::to_value(stage)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned()),
        batch_id: current_run.and_then(|run| run.batch_id.clone()),
        heartbeat_at_ms: unix_time_ms(),
    };
    let path = pipeline_owner_path(manifest_path);
    let temporary = path.with_extension("json.tmp");
    let mut bytes = serde_json::to_vec_pretty(&record).stringify_err()?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes).stringify_err()?;
    fs::rename(temporary, path).stringify_err()?;
    Ok(())
}

fn read_pipeline_owner_record(manifest_path: &Path) -> Option<PipelineOwnerRecord> {
    fs::read(pipeline_owner_path(manifest_path))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn process_is_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        pid == std::process::id()
    }
}

fn owner_record_is_live(record: &PipelineOwnerRecord, pipeline_id: &str, now_ms: u64) -> bool {
    record.pipeline_id == pipeline_id
        && record.state == "supervising"
        && now_ms.saturating_sub(record.heartbeat_at_ms) <= PIPELINE_OWNER_HEARTBEAT_FRESH_MS
        && process_is_alive(record.app_pid)
}

fn pipeline_has_live_owner_evidence(manifest: &PipelineManifest, manifest_path: &Path) -> bool {
    if active_pipeline_matches(&manifest.pipeline_id) {
        return true;
    }
    let current_batch = manifest
        .current_run_index
        .and_then(|index| manifest.runs.get(index))
        .and_then(|run| run.batch_id.as_deref());
    let capture = CaptureLock::global().snapshot();
    if capture.owner_kind.as_deref() == Some("pipeline")
        && capture.job_id.as_deref() == Some(manifest.pipeline_id.as_str())
    {
        return true;
    }
    if current_batch.is_some()
        && WorkerManager::global().active_job_id().as_deref() == current_batch
        && WorkerManager::global().state() == WorkerManagerState::Busy
    {
        return true;
    }
    read_pipeline_owner_record(manifest_path)
        .as_ref()
        .is_some_and(|record| owner_record_is_live(record, &manifest.pipeline_id, unix_time_ms()))
}

static UI_HEARTBEAT: OnceLock<UiHeartbeatState> = OnceLock::new();

fn ui_heartbeat_state() -> &'static UiHeartbeatState {
    UI_HEARTBEAT.get_or_init(|| UiHeartbeatState {
        active: AtomicBool::new(false),
        last_seen_ms: AtomicU64::new(0),
        warned: AtomicBool::new(false),
        monitor_started: AtomicBool::new(false),
    })
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn start_ui_heartbeat_monitor(state: &'static UiHeartbeatState) {
    if state.monitor_started.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if !state.active.load(Ordering::Acquire) {
                state.warned.store(false, Ordering::Release);
                continue;
            }
            let age = unix_time_ms().saturating_sub(state.last_seen_ms.load(Ordering::Acquire));
            if age >= UI_HEARTBEAT_WARN_AFTER_MS {
                if !state.warned.swap(true, Ordering::AcqRel) {
                    logging!(
                        warn,
                        Type::System,
                        "TrafficTracer UI heartbeat stalled for {} ms; capture and core were left running",
                        age
                    );
                }
            } else if state.warned.swap(false, Ordering::AcqRel) {
                logging!(info, Type::System, "TrafficTracer UI heartbeat recovered after a stall");
            }
        }
    });
}

#[tauri::command]
pub async fn tt_ui_heartbeat(active: bool) -> CmdResult<()> {
    let state = ui_heartbeat_state();
    state.active.store(active, Ordering::Release);
    if active {
        state.last_seen_ms.store(unix_time_ms(), Ordering::Release);
        start_ui_heartbeat_monitor(state);
    } else {
        state.warned.store(false, Ordering::Release);
    }
    Ok(())
}

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
    tt_get_environment_for_owner(app_handle, request, None).await
}

async fn tt_get_environment_for_owner(
    app_handle: AppHandle,
    request: EnvironmentRequest,
    pipeline_owner: Option<&str>,
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
        .ensure_session_root(
            &app_handle,
            requested_root,
            &controller_endpoint,
            &controller_secret,
            pipeline_owner,
        )
        .await
        .stringify_err()?;
    let active_root_string = active_root.to_string_lossy().into_owned();
    if pipeline_owner.is_none() {
        feat::patch_verge(
            &IVerge {
                traffic_tracer_output_root: Some(active_root_string.clone().into()),
                ..IVerge::default()
            },
            false,
        )
        .await
        .stringify_err()?;
    }
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
    #[serde(default)]
    pub playback: Option<PlaybackPolicy>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureNetwork {
    Tcp,
    Udp,
    All,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybackPolicy {
    pub provider: String,
    pub ad_policy: String,
    pub desired_primary_seconds: u32,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback: Option<PlaybackPolicy>,
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
    pub pcap_split_mode: String,
    pub cache_mode: String,
    pub proxy_protocol_mode: String,
    pub expected_proxy_protocol: String,
    pub proxy_selection_group: String,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            capture_packets: true,
            collect_cdp: true,
            collect_netlog: true,
            analyze_after_capture: true,
            headless: false,
            pcap_split_mode: "unique_connections".to_string(),
            cache_mode: "cold".to_string(),
            proxy_protocol_mode: "observe".to_string(),
            expected_proxy_protocol: String::new(),
            proxy_selection_group: String::new(),
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
    pub interrupt_requested: bool,
    #[serde(default)]
    pub cancel_requested_now: Option<bool>,
    #[serde(default)]
    pub interrupt_requested_now: Option<bool>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    playback: Option<PlaybackPolicy>,
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
                    playback: request.playback,
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
        || target.playback != request.playback
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
pub struct PipelineCurrentCandidateRequest {
    pub profile_uid: String,
    pub selection_group: String,
    pub requested_node: String,
}

#[tauri::command]
pub async fn tt_pipeline_current_candidate(request: PipelineCurrentCandidateRequest) -> CmdResult<PipelineCandidate> {
    CaptureLock::global()
        .ensure_unlocked("adding a profile and proxy pipeline candidate")
        .stringify_err()?;
    for (label, value) in [
        ("profile_uid", request.profile_uid.as_str()),
        ("selection_group", request.selection_group.as_str()),
        ("requested_node", request.requested_node.as_str()),
    ] {
        if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(format!("pipeline {label} is invalid").into());
        }
    }

    let profiles = Config::profiles().await.latest_arc();
    if profiles.current.as_deref() != Some(request.profile_uid.as_str()) {
        return Err("pipeline candidate must match the currently active Profile".into());
    }
    profiles
        .get_item(request.profile_uid.as_str())
        .map_err(|error| error.to_string())?;

    let proxies = handle::Handle::mihomo()
        .await
        .get_proxies()
        .await
        .map_err(|error| format!("CONTROLLER_UNAVAILABLE: {error}"))?;
    let group = proxies.proxies.get(request.selection_group.as_str()).ok_or_else(|| {
        smartstring::alias::String::from("pipeline selector group is not present in the active runtime")
    })?;
    if group.now.as_deref() != Some(request.requested_node.as_str()) {
        return Err("pipeline requested node is not the selector's current node".into());
    }
    if !group
        .all
        .as_ref()
        .is_some_and(|nodes| nodes.iter().any(|node| node.as_str() == request.requested_node))
    {
        return Err("pipeline requested node is not selectable from the runtime group".into());
    }

    let profile_fingerprint = effective_runtime_fingerprint()?;
    Ok(PipelineCandidate {
        profile_uid: request.profile_uid,
        profile_fingerprint,
        profile_fingerprint_kind: PIPELINE_FINGERPRINT_SEMANTIC_V2.into(),
        recorded_at: Some(Utc::now()),
        selection_group: request.selection_group,
        requested_node: request.requested_node,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    #[serde(default)]
    pub application_retry: ApplicationRetryPolicy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationRetryPolicy {
    pub enabled: bool,
    pub max_retries: u8,
}

impl Default for ApplicationRetryPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_retries: 1,
        }
    }
}

#[derive(Clone, Serialize)]
struct PipelineOrchestration {
    pipeline_id: String,
    run_id: String,
    run_ordinal: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    repetition_index: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_ordinal: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_position: Option<usize>,
    application_retry_attempt: u8,
    profile_uid: String,
    selection_group: String,
    requested_node: String,
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
    application_retry: ApplicationRetryPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    orchestration: Option<PipelineOrchestration>,
}

async fn validate_pipeline_batch_contract(
    manifest: &PipelineManifest,
    batch_template: &BatchStartRequest,
) -> CmdResult {
    let manager = WorkerManager::global();
    let client = manager.client().stringify_err()?;
    let secret = Config::clash()
        .await
        .latest_arc()
        .get_client_info()
        .secret
        .unwrap_or_default();

    let mut validated_targets = HashSet::new();
    for run in &manifest.runs {
        let target_index = run.target_index.ok_or_else(|| {
            smartstring::alias::String::from("PIPELINE_TARGET_SNAPSHOT_MISSING: matrix run has no target")
        })?;
        if !validated_targets.insert(target_index) {
            continue;
        }
        let target = batch_template
            .targets
            .iter()
            .find(|target| target.index == target_index)
            .cloned()
            .ok_or_else(|| {
                smartstring::alias::String::from("PIPELINE_TARGET_SNAPSHOT_MISSING: matrix target is absent")
            })?;
        let mut options = batch_template.options.clone();
        options.analyze_after_capture = false;
        options.proxy_selection_group = run.selection_group.clone();
        options.expected_proxy_protocol.clear();
        let candidate_position = manifest
            .schedule
            .repetition_candidate_orders
            .get(usize::from(run.repetition_index.saturating_sub(1)))
            .and_then(|order| order.iter().position(|ordinal| *ordinal == run.candidate_ordinal))
            .map(|position| position + 1);
        let job_id = new_job_id()?;
        let result = client
            .request::<_, Value>(
                RequestMethod::BatchValidate,
                BatchJobParams {
                    job: BatchJobSpec {
                        schema_version: JOB_SCHEMA_VERSION,
                        kind: "batch",
                        job_id,
                        config_path: batch_template.config_path.clone(),
                        config_sha256: batch_template.config_sha256.clone(),
                        targets: vec![target],
                        interfaces: CaptureInterfaces {
                            tun: batch_template.tun_interface.clone(),
                            physical: batch_template.physical_interface.clone(),
                        },
                        output_root: batch_template.output_root.clone(),
                        chrome_binary: batch_template.chrome_binary.clone(),
                        controller: CaptureController {
                            endpoint: local_controller_endpoint(),
                            secret: secret.clone(),
                        },
                        options,
                        fail_fast: batch_template.fail_fast,
                        application_retry: ApplicationRetryPolicy {
                            enabled: false,
                            max_retries: 1,
                        },
                        orchestration: Some(PipelineOrchestration {
                            pipeline_id: manifest.pipeline_id.clone(),
                            run_id: run.run_id.clone(),
                            run_ordinal: run.ordinal,
                            repetition_index: Some(run.repetition_index),
                            target_index: run.target_index,
                            candidate_ordinal: Some(run.candidate_ordinal),
                            candidate_position,
                            application_retry_attempt: run.application_retry_attempt,
                            profile_uid: run.profile_uid.clone(),
                            selection_group: run.selection_group.clone(),
                            requested_node: run.requested_node.clone(),
                        }),
                    },
                },
            )
            .await
            .map_err(|error| format!("PIPELINE_BATCH_PREFLIGHT_FAILED: {error}"))?;
        if result.get("valid").and_then(Value::as_bool) != Some(true) {
            return Err("PIPELINE_BATCH_PREFLIGHT_FAILED: Worker rejected validation without an explicit error".into());
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct BatchIdParams {
    batch_id: String,
}

#[derive(Serialize)]
struct BatchStopParams {
    batch_id: String,
    reason: String,
}

#[tauri::command]
pub async fn tt_batch_start(app_handle: AppHandle, request: BatchStartRequest) -> CmdResult<JobSnapshot> {
    tt_batch_start_for_owner(app_handle, request, None, None).await
}

async fn tt_batch_start_for_owner(
    app_handle: AppHandle,
    request: BatchStartRequest,
    pipeline_owner: Option<&str>,
    orchestration: Option<PipelineOrchestration>,
) -> CmdResult<JobSnapshot> {
    validate_batch_start_request_for_owner(&request, pipeline_owner.is_some())?;
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
    let environment = tt_get_environment_for_owner(
        app_handle,
        EnvironmentRequest {
            tun_interface: request.tun_interface.clone(),
            physical_interface: request.physical_interface.clone(),
            chrome_binary: request.chrome_binary.clone(),
            output_root: request.output_root.clone(),
            min_free_bytes: None,
        },
        pipeline_owner,
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
    match pipeline_owner {
        Some(owner) => lock
            .ensure_owned("pipeline", owner, "starting a pipeline batch")
            .stringify_err()?,
        None => lock
            .acquire(job_id.clone(), "TrafficTracer batch capture is active")
            .stringify_err()?,
    }
    if let Err(error) = manager.mark_busy(&job_id) {
        if pipeline_owner.is_none() {
            let _ = lock.release(&job_id);
        }
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
                    application_retry: request.application_retry,
                    orchestration,
                },
            },
        )
        .await;
    finish_batch_request(result, &job_id, manager, pipeline_owner.is_none()).await
}

fn validate_batch_start_request(request: &BatchStartRequest) -> CmdResult {
    validate_batch_start_request_for_owner(request, false)
}

fn validate_batch_start_request_for_owner(request: &BatchStartRequest, allow_deferred_analysis: bool) -> CmdResult {
    if request.targets.is_empty() {
        return Err("batch targets must not be empty".into());
    }
    if !request.options.analyze_after_capture && !allow_deferred_analysis {
        return Err("batch requires analysis after every capture".into());
    }
    if request.application_retry.max_retries != 1 {
        return Err("application retry must be bounded to exactly one retry".into());
    }
    if !valid_pcap_split_mode(&request.options.pcap_split_mode) {
        return Err("pcap_split_mode must be none or unique_connections".into());
    }
    if !valid_cache_mode(&request.options.cache_mode) {
        return Err("cache_mode must be cold or warm".into());
    }
    if !valid_proxy_protocol_mode(&request.options.proxy_protocol_mode) {
        return Err("proxy_protocol_mode must be strict_single or observe".into());
    }
    if !valid_proxy_protocol(&request.options.expected_proxy_protocol) {
        return Err("expected_proxy_protocol must be a protocol name".into());
    }
    if request.options.proxy_selection_group.chars().any(char::is_control) {
        return Err("proxy_selection_group must not contain control characters".into());
    }
    Ok(())
}

async fn finish_batch_request(
    result: Result<JobSnapshot, crate::core::traffic_tracer::client::ClientError>,
    job_id: &str,
    manager: &WorkerManager,
    release_capture_lock: bool,
) -> CmdResult<JobSnapshot> {
    match result {
        Ok(snapshot) => {
            if snapshot.state.terminal() {
                if release_capture_lock {
                    let _ = CaptureLock::global().release(job_id);
                }
                let _ = manager.mark_ready(job_id);
            }
            Ok(snapshot)
        }
        Err(error) => {
            if batch_request_outcome_unknown(&error) {
                if let Ok(client) = manager.client() {
                    if let Ok(status) = client
                        .request::<_, Value>(
                            RequestMethod::BatchStatus,
                            BatchIdParams {
                                batch_id: job_id.to_owned(),
                            },
                        )
                        .await
                    {
                        if let Some(job) = status.get("job").filter(|value| !value.is_null()) {
                            if let Ok(snapshot) = serde_json::from_value::<JobSnapshot>(job.clone()) {
                                return Ok(snapshot);
                            }
                        }
                        if status.get("batch").is_some_and(|value| !value.is_null()) {
                            return Ok(uncertain_batch_start_snapshot(job_id));
                        }
                    }
                    if let Ok(snapshot) = client
                        .request::<_, JobSnapshot>(
                            RequestMethod::JobStatus,
                            JobIdParams {
                                job_id: job_id.to_owned(),
                            },
                        )
                        .await
                    {
                        return Ok(snapshot);
                    }
                }
                // A timeout, lost response, decode failure, or Worker exit does not
                // prove rejection. Retain capture ownership and let the supervisor
                // reconcile the pre-generated Job ID instead of publishing a false
                // terminal failure while a Batch may still be active.
                return Ok(uncertain_batch_start_snapshot(job_id));
            }
            if release_capture_lock {
                let _ = CaptureLock::global().release(job_id);
            }
            let _ = manager.mark_ready(job_id);
            Err(error.to_string().into())
        }
    }
}

fn batch_request_outcome_unknown(error: &crate::core::traffic_tracer::client::ClientError) -> bool {
    use crate::core::traffic_tracer::client::ClientError;

    matches!(
        error,
        ClientError::Decode(_)
            | ClientError::Protocol(_)
            | ClientError::Transport(_)
            | ClientError::Timeout(_)
            | ClientError::WorkerExited
    )
}

fn uncertain_batch_start_snapshot(job_id: &str) -> JobSnapshot {
    JobSnapshot {
        job_id: job_id.to_owned(),
        kind: "batch".to_owned(),
        state: JobState::Created,
        stage: "starting_batch".to_owned(),
        progress: 0.0,
        message: "Batch start outcome is being reconciled.".to_owned(),
        cancel_requested: false,
        interrupt_requested: false,
        cancel_requested_now: None,
        interrupt_requested_now: None,
        result: None,
        error: None,
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

fn pipeline_default_continue() -> bool {
    true
}

fn pipeline_default_repetitions() -> u16 {
    1
}
fn pipeline_default_order_policy() -> PipelineCandidateOrderPolicy {
    PipelineCandidateOrderPolicy::BalancedSeeded
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineStartRequest {
    pub batch: BatchStartRequest,
    pub candidates: Vec<PipelineCandidate>,
    #[serde(default = "pipeline_default_repetitions")]
    pub repetitions_per_candidate: u16,
    #[serde(default = "pipeline_default_continue")]
    pub continue_on_run_failure: bool,
    #[serde(default = "pipeline_default_order_policy")]
    pub candidate_order_policy: PipelineCandidateOrderPolicy,
    #[serde(default)]
    pub random_seed: Option<u64>,
}

fn pipeline_execution_snapshot(batch: &BatchStartRequest) -> Value {
    serde_json::json!({ "tun_interface": batch.tun_interface, "physical_interface": batch.physical_interface, "chrome_binary": batch.chrome_binary, "options": batch.options, "fail_fast": batch.fail_fast, "application_retry": batch.application_retry })
}

fn pipeline_target(target: &TargetConfigEntry) -> PipelineTarget {
    PipelineTarget {
        index: target.index,
        url: target.url.clone(),
        domain: target.domain.clone(),
        duration_seconds: u64::from(target.duration_seconds),
        network: match target.network {
            CaptureNetwork::Tcp => "tcp",
            CaptureNetwork::Udp => "udp",
            CaptureNetwork::All => "all",
        }
        .to_owned(),
        run_label: target.run_label.clone(),
        wait_load_timeout: u64::from(target.wait_load_timeout),
        page_type: target.page_type.clone(),
        playback: target
            .playback
            .as_ref()
            .and_then(|value| serde_json::to_value(value).ok()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PipelineExecutionSnapshot {
    tun_interface: String,
    physical_interface: String,
    chrome_binary: String,
    options: CaptureOptions,
    fail_fast: bool,
    #[serde(default)]
    application_retry: ApplicationRetryPolicy,
}

fn pipeline_batch_from_manifest(manifest: &PipelineManifest) -> Result<BatchStartRequest, String> {
    let execution: PipelineExecutionSnapshot = serde_json::from_value(manifest.execution.clone())
        .map_err(|error| format!("PIPELINE_EXECUTION_INVALID: {error}"))?;
    let targets = manifest
        .targets
        .iter()
        .map(|target| {
            let network = match target.network.as_str() {
                "tcp" => CaptureNetwork::Tcp,
                "udp" => CaptureNetwork::Udp,
                "all" => CaptureNetwork::All,
                value => return Err(format!("PIPELINE_TARGET_INVALID: unknown network {value}")),
            };
            let playback = target
                .playback
                .clone()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| format!("PIPELINE_TARGET_INVALID: {error}"))?;
            Ok(TargetConfigEntry {
                index: target.index,
                domain: target.domain.clone(),
                url: target.url.clone(),
                duration_seconds: target
                    .duration_seconds
                    .try_into()
                    .map_err(|_| "PIPELINE_TARGET_INVALID: duration_seconds is too large".to_owned())?,
                network,
                run_label: target.run_label.clone(),
                wait_load_timeout: target
                    .wait_load_timeout
                    .try_into()
                    .map_err(|_| "PIPELINE_TARGET_INVALID: wait_load_timeout is too large".to_owned())?,
                page_type: target.page_type.clone(),
                playback,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(BatchStartRequest {
        config_path: manifest.config.path.to_string_lossy().into_owned(),
        config_sha256: manifest.config.sha256.clone(),
        targets,
        tun_interface: execution.tun_interface,
        physical_interface: execution.physical_interface,
        output_root: manifest.output_root.to_string_lossy().into_owned(),
        chrome_binary: execution.chrome_binary,
        options: execution.options,
        fail_fast: execution.fail_fast,
        application_retry: execution.application_retry,
    })
}

#[derive(Clone, Debug)]
struct PipelineBarrierError {
    code: &'static str,
    message: String,
}

impl PipelineBarrierError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn render(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}

fn normalize_proxy_protocol(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn proxy_snapshot_from_runtime(
    proxies: &Proxies,
    profile_uid: String,
    profile_fingerprint: String,
    selection_group: &str,
) -> Result<PipelineProxySnapshot, PipelineBarrierError> {
    let group = proxies
        .proxies
        .get(selection_group)
        .ok_or_else(|| PipelineBarrierError::new("SELECTOR_NOT_FOUND", "selector is absent from the active runtime"))?;
    let selected_node = group.now.clone().ok_or_else(|| {
        PipelineBarrierError::new(
            "SELECTOR_READBACK_UNAVAILABLE",
            "selector does not expose its current node",
        )
    })?;
    let mut resolved_chain = vec![selected_node.clone()];
    let mut resolved_leaf = selected_node.clone();
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(resolved_leaf.clone()) {
            return Err(PipelineBarrierError::new(
                "PROXY_CHAIN_CYCLE",
                "selected proxy chain contains a cycle",
            ));
        }
        let proxy = proxies.proxies.get(resolved_leaf.as_str()).ok_or_else(|| {
            PipelineBarrierError::new(
                "PROXY_LEAF_NOT_FOUND",
                "selected node is absent from the active runtime",
            )
        })?;
        let Some(next) = proxy.now.as_ref() else {
            let protocol = serde_json::to_value(&proxy.proxy_type)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .map(|value| normalize_proxy_protocol(&value))
                .unwrap_or_else(|| "unknown".into());
            return Ok(PipelineProxySnapshot {
                profile_uid,
                profile_fingerprint,
                selection_group: selection_group.to_owned(),
                selected_node,
                resolved_chain,
                resolved_leaf,
                protocol,
                captured_at: Utc::now(),
            });
        };
        resolved_leaf = next.clone();
        resolved_chain.push(resolved_leaf.clone());
        if resolved_chain.len() > 16 {
            return Err(PipelineBarrierError::new(
                "PROXY_CHAIN_TOO_DEEP",
                "selected proxy chain exceeds 16 hops",
            ));
        }
    }
}

async fn read_pipeline_proxy_snapshot(selection_group: &str) -> Result<PipelineProxySnapshot, PipelineBarrierError> {
    let profile_uid = Config::profiles()
        .await
        .latest_arc()
        .current
        .clone()
        .map(String::from)
        .ok_or_else(|| PipelineBarrierError::new("PROFILE_READBACK_UNAVAILABLE", "no active Profile is reported"))?;
    let profile_fingerprint = effective_runtime_fingerprint()
        .map_err(|error| PipelineBarrierError::new("PROFILE_FINGERPRINT_UNAVAILABLE", error.to_string()))?;
    let proxies = handle::Handle::mihomo()
        .await
        .get_proxies()
        .await
        .map_err(|error| PipelineBarrierError::new("CONTROLLER_UNAVAILABLE", error.to_string()))?;
    proxy_snapshot_from_runtime(&proxies, profile_uid, profile_fingerprint, selection_group)
}

async fn wait_for_profile_controller(
    expected_profile: &str,
    expected_fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<(), PipelineBarrierError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let current = Config::profiles().await.latest_arc().current.clone().map(String::from);
        let current_error = if current.as_deref() != Some(expected_profile) {
            PipelineBarrierError::new(
                "PROFILE_READBACK_MISMATCH",
                format!(
                    "expected Profile {expected_profile}; observed {}",
                    current.as_deref().unwrap_or("none")
                ),
            )
        } else {
            match effective_runtime_fingerprint() {
                Ok(fingerprint) if expected_fingerprint.is_none_or(|expected| fingerprint == expected) => {
                    match handle::Handle::mihomo().await.get_proxies().await {
                        Ok(_) => return Ok(()),
                        Err(error) => PipelineBarrierError::new("CONTROLLER_UNAVAILABLE", error.to_string()),
                    }
                }
                Ok(fingerprint) => PipelineBarrierError::new(
                    "PROFILE_FINGERPRINT_MISMATCH",
                    format!("effective runtime fingerprint is {fingerprint}"),
                ),
                Err(error) => PipelineBarrierError::new("PROFILE_FINGERPRINT_UNAVAILABLE", error.to_string()),
            }
        };
        if tokio::time::Instant::now() >= deadline {
            return Err(current_error);
        }
        tokio::time::sleep(PIPELINE_POLL_INTERVAL).await;
    }
}

async fn wait_for_selected_snapshot(
    expected_profile: &str,
    expected_fingerprint: &str,
    selection_group: &str,
    expected_node: &str,
    timeout: Duration,
) -> Result<PipelineProxySnapshot, PipelineBarrierError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let current_error = match read_pipeline_proxy_snapshot(selection_group).await {
            Ok(snapshot)
                if snapshot.profile_uid == expected_profile
                    && snapshot.profile_fingerprint == expected_fingerprint
                    && snapshot.selected_node == expected_node =>
            {
                return Ok(snapshot);
            }
            Ok(snapshot) => {
                let (code, message) = if snapshot.profile_uid != expected_profile {
                    (
                        "PROFILE_READBACK_MISMATCH",
                        format!("expected Profile {expected_profile}; observed {}", snapshot.profile_uid),
                    )
                } else if snapshot.profile_fingerprint != expected_fingerprint {
                    (
                        "PROFILE_FINGERPRINT_MISMATCH",
                        "effective runtime changed after the candidate was queued".into(),
                    )
                } else {
                    (
                        "NODE_READBACK_MISMATCH",
                        format!("expected node {expected_node}; observed {}", snapshot.selected_node),
                    )
                };
                PipelineBarrierError::new(code, message)
            }
            Err(error) => error,
        };
        if tokio::time::Instant::now() >= deadline {
            return Err(current_error);
        }
        tokio::time::sleep(PIPELINE_POLL_INTERVAL).await;
    }
}

async fn wait_for_end_snapshot(
    selection_group: &str,
    expected_profile: &str,
    expected_fingerprint: &str,
    timeout: Duration,
) -> Result<PipelineProxySnapshot, PipelineBarrierError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let current_error = match read_pipeline_proxy_snapshot(selection_group).await {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) => {
                let current = Config::profiles().await.latest_arc().current.clone().map(String::from);
                if current.as_deref() != Some(expected_profile) {
                    PipelineBarrierError::new(
                        "PROFILE_READBACK_MISMATCH",
                        format!(
                            "expected Profile {expected_profile}; observed {}",
                            current.as_deref().unwrap_or("none")
                        ),
                    )
                } else if effective_runtime_fingerprint().is_ok_and(|value| value != expected_fingerprint) {
                    PipelineBarrierError::new(
                        "PROFILE_FINGERPRINT_MISMATCH",
                        "effective runtime changed during the Batch",
                    )
                } else {
                    error
                }
            }
        };
        if tokio::time::Instant::now() >= deadline {
            return Err(current_error);
        }
        tokio::time::sleep(PIPELINE_POLL_INTERVAL).await;
    }
}

fn proxy_snapshots_match(left: &PipelineProxySnapshot, right: &PipelineProxySnapshot) -> bool {
    left.profile_uid == right.profile_uid
        && left.profile_fingerprint == right.profile_fingerprint
        && left.selection_group == right.selection_group
        && left.selected_node == right.selected_node
        && left.resolved_chain == right.resolved_chain
        && left.resolved_leaf == right.resolved_leaf
        && left.protocol == right.protocol
}

fn requested_pipeline_stop(interrupt: &AtomicBool, cancel: &AtomicBool) -> Option<PipelineRunState> {
    if cancel.load(Ordering::Acquire) {
        Some(PipelineRunState::Cancelled)
    } else if interrupt.load(Ordering::Acquire) {
        Some(PipelineRunState::Interrupted)
    } else {
        None
    }
}

async fn observe_pipeline_connections() -> PipelineConnectionDrain {
    match handle::Handle::mihomo().await.get_connections().await {
        Ok(value) => {
            let count = value.connections.unwrap_or_default().len();
            PipelineConnectionDrain {
                state: "preserved".into(),
                initial_connections: Some(count),
                final_connections: Some(count),
                polls: 1,
                quiet_millis: 0,
                error: None,
                completed_at: Utc::now(),
            }
        }
        Err(error) => PipelineConnectionDrain {
            state: "controller_unavailable".into(),
            initial_connections: None,
            final_connections: None,
            polls: 1,
            quiet_millis: 0,
            error: Some(error.to_string()),
            completed_at: Utc::now(),
        },
    }
}

#[derive(Default)]
struct ObservedRunEvidence {
    protocols: std::collections::BTreeSet<String>,
    selected_nodes: std::collections::BTreeSet<String>,
    leaf_nodes: std::collections::BTreeSet<String>,
    contexts: usize,
}

fn observed_run_evidence(output_path: &Path) -> ObservedRunEvidence {
    fn scan(path: &Path, depth: usize, evidence: &mut ObservedRunEvidence) {
        if depth > 8 {
            return;
        }
        let Ok(entries) = fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan(&path, depth + 1, evidence);
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) != Some("capture-context.json") {
                continue;
            }
            let Ok(value) = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .ok_or(())
            else {
                continue;
            };
            evidence.contexts += 1;
            if let Some(items) = value
                .pointer("/proxy_protocol/runtime_observation/protocols")
                .and_then(Value::as_array)
            {
                evidence.protocols.extend(
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(normalize_proxy_protocol)
                        .filter(|value| !value.is_empty()),
                );
            }
            if let Some(node) = value
                .pointer("/proxy_protocol/selected_scope/node")
                .and_then(Value::as_str)
            {
                evidence.selected_nodes.insert(node.to_owned());
            }
            if let Some(node) = value
                .pointer("/proxy_protocol/selected_scope/leaf_node")
                .and_then(Value::as_str)
            {
                evidence.leaf_nodes.insert(node.to_owned());
            }
        }
    }
    let mut evidence = ObservedRunEvidence::default();
    scan(output_path, 0, &mut evidence);
    evidence
}

fn observed_run_protocol(evidence: &ObservedRunEvidence) -> String {
    match evidence.protocols.len() {
        0 => String::new(),
        1 => evidence.protocols.iter().next().cloned().unwrap_or_default(),
        _ => "mixed".to_owned(),
    }
}

fn classify_run_verification(
    start: &PipelineProxySnapshot,
    end: Option<&PipelineProxySnapshot>,
    observed: &ObservedRunEvidence,
    end_error: Option<&PipelineBarrierError>,
) -> PipelineRunVerification {
    let mut details = Vec::new();
    let mut node_state = "passed";
    match end {
        Some(snapshot) if !proxy_snapshots_match(start, snapshot) => {
            node_state = "node_drift";
            details.push("Controller selection or resolved chain changed during the Batch".into());
        }
        None => {
            node_state = if end_error.is_some_and(|error| {
                matches!(
                    error.code,
                    "PROFILE_READBACK_MISMATCH" | "PROFILE_FINGERPRINT_MISMATCH" | "NODE_READBACK_MISMATCH"
                )
            }) {
                "node_drift"
            } else {
                "observation_unavailable"
            };
            details.push(
                end_error
                    .map(PipelineBarrierError::render)
                    .unwrap_or_else(|| "end-of-run Controller snapshot is unavailable".into()),
            );
        }
        _ => {}
    }
    if observed.contexts == 0 || observed.selected_nodes.is_empty() {
        if node_state == "passed" {
            node_state = "observation_unavailable";
        }
        details.push("Session proxy selection snapshots are unavailable".into());
    } else if observed.selected_nodes.len() != 1
        || !observed.selected_nodes.contains(&start.selected_node)
        || (!observed.leaf_nodes.is_empty()
            && (observed.leaf_nodes.len() != 1 || !observed.leaf_nodes.contains(&start.resolved_leaf)))
    {
        node_state = "node_drift";
        details.push("Session snapshots do not agree with the frozen node chain".into());
    }

    let expected_protocol = normalize_proxy_protocol(&start.protocol);
    let mut protocol_state =
        if observed.protocols.is_empty() || expected_protocol.is_empty() || expected_protocol == "unknown" {
            "observation_unavailable"
        } else if observed.protocols.len() == 1 && observed.protocols.contains(&expected_protocol) {
            "passed"
        } else {
            "protocol_mismatch"
        };
    if let Some(snapshot) = end
        && normalize_proxy_protocol(&snapshot.protocol) != expected_protocol
    {
        protocol_state = "protocol_mismatch";
        details.push("end-of-run leaf protocol differs from the frozen protocol".into());
    }
    if protocol_state == "observation_unavailable" {
        details.push("bounded Mihomo trace contains no proxy protocol observation".into());
    } else if protocol_state == "protocol_mismatch" {
        details.push("bounded Mihomo trace protocol differs from the frozen protocol".into());
    }

    PipelineRunVerification {
        node_state: node_state.into(),
        protocol_state: protocol_state.into(),
        observed_protocols: observed.protocols.iter().cloned().collect(),
        observed_selected_nodes: observed.selected_nodes.iter().cloned().collect(),
        observed_leaf_nodes: observed.leaf_nodes.iter().cloned().collect(),
        details,
        checked_at: Utc::now(),
    }
}

fn verification_requires_attention(verification: &PipelineRunVerification) -> bool {
    verification.node_state != "passed" || verification.protocol_state != "passed"
}

fn pipeline_run_error(message: String) -> PipelineError {
    let code = message
        .split_once(':')
        .map(|(code, _)| code)
        .filter(|code| {
            !code.is_empty()
                && code.len() <= 96
                && code
                    .chars()
                    .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_')
        })
        .unwrap_or("PIPELINE_RUN_FAILED");
    PipelineError {
        code: code.into(),
        message,
    }
}

#[derive(Default)]
struct QualityCounts {
    passed: usize,
    degraded: usize,
    failed: usize,
    indeterminate: usize,
    not_applicable: usize,
}

impl QualityCounts {
    fn observe(&mut self, state: Option<&str>, applicable: bool) {
        if !applicable {
            self.not_applicable += 1;
            return;
        }
        match state {
            Some("passed" | "good") => self.passed += 1,
            Some("not_applicable") => self.not_applicable += 1,
            Some("degraded") => self.degraded += 1,
            Some("failed" | "unavailable") => self.failed += 1,
            _ => self.indeterminate += 1,
        }
    }

    fn finish(self) -> PipelineQualityPlane {
        let state = if self.failed > 0 {
            "failed"
        } else if self.degraded > 0 {
            "degraded"
        } else if self.indeterminate > 0 {
            "indeterminate"
        } else if self.passed > 0 {
            "passed"
        } else {
            "not_applicable"
        };
        PipelineQualityPlane {
            state: state.into(),
            passed: self.passed,
            degraded: self.degraded,
            failed: self.failed,
            indeterminate: self.indeterminate,
            not_applicable: self.not_applicable,
        }
    }
}

fn run_quality_requires_attention(quality: &PipelineRunQuality) -> bool {
    matches!(
        quality.capture_integrity.state.as_str(),
        "failed" | "degraded" | "indeterminate"
    ) || matches!(
        quality.correlation.state.as_str(),
        "failed" | "degraded" | "indeterminate"
    ) || matches!(
        quality.application.state.as_str(),
        "failed" | "degraded" | "indeterminate"
    )
}

fn pipeline_run_quality(output_path: &Path, effective_sessions: Option<&HashSet<String>>) -> PipelineRunQuality {
    fn scan(path: &Path, depth: usize, summaries: &mut Vec<PathBuf>) {
        if depth > 8 {
            return;
        }
        let Ok(entries) = fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan(&path, depth + 1, summaries);
            } else if path.file_name().and_then(|name| name.to_str()) == Some("summary.json")
                && path.parent().and_then(Path::file_name).and_then(|name| name.to_str()) == Some("analysis")
            {
                summaries.push(path);
            }
        }
    }

    fn string_at(value: &Value, pointer: &str) -> Option<String> {
        value.pointer(pointer).and_then(Value::as_str).map(str::to_owned)
    }

    let mut summaries = Vec::new();
    scan(output_path, 0, &mut summaries);
    summaries.sort();
    let mut capture = QualityCounts::default();
    let mut correlation = QualityCounts::default();
    let mut application = QualityCounts::default();
    let mut application_issues = Vec::new();
    let mut sessions_total = 0;

    for path in &summaries {
        let value = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let Some(value) = value else {
            if effective_sessions.is_some() {
                continue;
            }
            sessions_total += 1;
            capture.observe(None, true);
            correlation.observe(None, true);
            application.observe(None, true);
            continue;
        };
        if effective_sessions.is_some_and(|session_ids| {
            value
                .get("session_id")
                .and_then(Value::as_str)
                .is_none_or(|session_id| !session_ids.contains(session_id))
        }) {
            continue;
        }
        sessions_total += 1;
        capture.observe(
            value
                .pointer("/analysis_integrity/page_attributed/state")
                .and_then(Value::as_str),
            true,
        );
        correlation.observe(value.get("quality_state").and_then(Value::as_str), true);

        let scenario = value
            .get("activity_outcome")
            .filter(|item| item.is_object())
            .or_else(|| value.get("scenario_outcome").filter(|item| item.is_object()));
        application.observe(
            scenario.and_then(|item| item.get("state")).and_then(Value::as_str),
            scenario.is_some(),
        );
        let scenario_state = scenario.and_then(|item| item.get("state")).and_then(Value::as_str);
        if scenario.is_none() || scenario_state == Some("passed") || scenario_state == Some("not_applicable") {
            continue;
        }

        let session_dir = path.parent().and_then(Path::parent);
        let context = session_dir
            .and_then(|dir| fs::read(dir.join("raw/capture-context.json")).ok())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(Value::Null);
        let primary_content_millis = scenario
            .and_then(|item| item.get("primary_content_seconds"))
            .and_then(Value::as_f64)
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .map(|seconds| (seconds * 1000.0).round() as u64);
        application_issues.push(PipelineApplicationIssue {
            session_id: string_at(&value, "/session_id").unwrap_or_default(),
            target_url: string_at(&context, "/target/url").unwrap_or_default(),
            final_url: string_at(&value, "/activity_outcome/final_url")
                .or_else(|| string_at(&value, "/playback/diagnostics/last_observation/href")),
            final_status: value
                .pointer("/activity_outcome/final_status")
                .and_then(Value::as_u64)
                .and_then(|status| u16::try_from(status).ok()),
            state: scenario_state.unwrap_or("indeterminate").to_owned(),
            reason: scenario
                .and_then(|item| item.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            primary_content_millis,
            desired_primary_seconds: scenario
                .and_then(|item| item.get("desired_primary_seconds"))
                .and_then(Value::as_u64),
        });
    }

    if sessions_total == 0 {
        capture.observe(None, true);
        correlation.observe(None, true);
        application.observe(None, true);
    }
    PipelineRunQuality {
        sessions_total,
        capture_integrity: capture.finish(),
        correlation: correlation.finish(),
        application: application.finish(),
        application_issues,
    }
}

fn batch_effective_sessions(output_root: &Path, batch_id: &str) -> Option<HashSet<String>> {
    let path = output_root.join(".batches").join(batch_id).join("batch-manifest.json");
    let value = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())?;
    let sessions = value
        .get("children")?
        .as_array()?
        .iter()
        .filter_map(|child| child.get("session_id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    (!sessions.is_empty()).then_some(sessions)
}

fn canonical_yaml_bytes(value: &serde_yaml_ng::Value, output: &mut Vec<u8>) -> Result<(), String> {
    fn write_bytes(output: &mut Vec<u8>, tag: u8, bytes: &[u8]) {
        output.push(tag);
        output.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        output.extend_from_slice(bytes);
    }

    match value {
        serde_yaml_ng::Value::Null => output.push(b'n'),
        serde_yaml_ng::Value::Bool(value) => output.push(if *value { b't' } else { b'f' }),
        serde_yaml_ng::Value::Number(value) => write_bytes(output, b'#', value.to_string().as_bytes()),
        serde_yaml_ng::Value::String(value) => write_bytes(output, b'"', value.as_bytes()),
        serde_yaml_ng::Value::Sequence(values) => {
            output.push(b'[');
            output.extend_from_slice(&(values.len() as u64).to_be_bytes());
            for value in values {
                canonical_yaml_bytes(value, output)?;
            }
        }
        serde_yaml_ng::Value::Mapping(values) => {
            let mut entries = Vec::with_capacity(values.len());
            for (key, value) in values {
                let mut canonical_key = Vec::new();
                let mut canonical_value = Vec::new();
                canonical_yaml_bytes(key, &mut canonical_key)?;
                canonical_yaml_bytes(value, &mut canonical_value)?;
                entries.push((canonical_key, canonical_value));
            }
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            output.push(b'{');
            output.extend_from_slice(&(entries.len() as u64).to_be_bytes());
            for (key, value) in entries {
                write_bytes(output, b'k', &key);
                write_bytes(output, b'v', &value);
            }
        }
        serde_yaml_ng::Value::Tagged(value) => {
            write_bytes(output, b'!', value.tag.to_string().as_bytes());
            canonical_yaml_bytes(&value.value, output)?;
        }
    }
    Ok(())
}

fn semantic_runtime_fingerprint(runtime: &[u8]) -> Result<String, String> {
    let value: serde_yaml_ng::Value =
        serde_yaml_ng::from_slice(runtime).map_err(|error| format!("decode runtime YAML: {error}"))?;
    let mut canonical = Vec::new();
    canonical_yaml_bytes(&value, &mut canonical)?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

fn effective_runtime_fingerprint() -> CmdResult<String> {
    let path = crate::utils::dirs::app_home_dir()
        .stringify_err()?
        .join(crate::constants::files::RUNTIME_CONFIG);
    semantic_runtime_fingerprint(&fs::read(path).stringify_err()?).map_err(Into::into)
}

fn pipeline_directory(workspace_root: &str, pipeline_id: &str) -> CmdResult<PathBuf> {
    let root = PathBuf::from(workspace_root);
    if !root.is_absolute() {
        return Err("pipeline output_root must be absolute".into());
    }
    let prefix = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    Ok(root.join(format!(
        "{prefix}__pipeline-{}",
        &pipeline_id[..12.min(pipeline_id.len())]
    )))
}

fn launch_pipeline_supervisor(
    app_handle: AppHandle,
    manifest_path: PathBuf,
    pipeline_id: String,
    batch: BatchStartRequest,
) {
    let interrupt = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    *pipeline_runtime().active.lock() = Some(ActivePipeline {
        pipeline_id: pipeline_id.clone(),
        manifest_path: manifest_path.clone(),
        interrupt: Arc::clone(&interrupt),
        cancel: Arc::clone(&cancel),
    });
    let heartbeat_done = Arc::new(AtomicBool::new(false));
    let heartbeat_manifest = manifest_path.clone();
    let heartbeat_pipeline = pipeline_id.clone();
    let heartbeat_stop = Arc::clone(&heartbeat_done);
    let _ = write_pipeline_owner_record(&manifest_path, &pipeline_id, "supervising");
    tauri::async_runtime::spawn(async move {
        while !heartbeat_stop.load(Ordering::Acquire) {
            tokio::time::sleep(PIPELINE_OWNER_HEARTBEAT_INTERVAL).await;
            if heartbeat_stop.load(Ordering::Acquire) {
                break;
            }
            if let Err(error) = write_pipeline_owner_record(&heartbeat_manifest, &heartbeat_pipeline, "supervising") {
                logging!(
                    warn,
                    Type::System,
                    "TrafficTracer pipeline owner heartbeat failed: {error}"
                );
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run_pipeline(app_handle, manifest_path.clone(), batch, interrupt, cancel).await {
            logging!(error, Type::System, "TrafficTracer pipeline supervisor failed: {error}");
            if let Ok(mut failed) = PipelineManifest::load(&manifest_path) {
                if failed.current_run_index.is_some() {
                    let _ = failed.finish_run(
                        PipelineRunState::Failed,
                        Some(PipelineError {
                            code: "PIPELINE_SUPERVISOR_FAILED".into(),
                            message: error,
                        }),
                    );
                }
                failed.state = PipelineState::Failed;
                failed.stage = PipelineStage::Finished;
                failed.updated_at = Utc::now();
                let _ = failed.persist();
                restore_pipeline(&mut failed, &pipeline_id).await;
            }
        }
        let _ = CaptureLock::global().release(&pipeline_id);
        heartbeat_done.store(true, Ordering::Release);
        let _ = write_pipeline_owner_record(&manifest_path, &pipeline_id, "released");
        let mut active = pipeline_runtime().active.lock();
        if active.as_ref().is_some_and(|item| item.pipeline_id == pipeline_id) {
            *active = None;
        }
    });
}

#[tauri::command]
pub async fn tt_pipeline_start(
    app_handle: AppHandle,
    mut request: PipelineStartRequest,
) -> CmdResult<PipelineManifest> {
    if request.candidates.is_empty() {
        return Err("pipeline candidates must not be empty".into());
    }
    if !(1..=PIPELINE_MAX_REPETITIONS).contains(&request.repetitions_per_candidate) {
        return Err(
            format!("pipeline repetitions_per_candidate must be between 1 and {PIPELINE_MAX_REPETITIONS}").into(),
        );
    }
    validate_batch_start_request(&request.batch)?;
    let preview = tt_target_config_load(request.batch.config_path.clone()).await?;
    validate_batch_selection(&preview, &request.batch)?;
    let selected = request
        .batch
        .targets
        .iter()
        .map(|target| target.index)
        .collect::<std::collections::HashSet<_>>();
    request.batch.targets = preview
        .targets
        .iter()
        .filter(|target| selected.contains(&target.index))
        .cloned()
        .collect();

    let profiles = Config::profiles().await.latest_arc();
    for candidate in &request.candidates {
        profiles
            .get_item(candidate.profile_uid.as_str())
            .map_err(|error| format!("PIPELINE_PROFILE_NOT_FOUND: {error}"))?;
    }
    let active_profile = profiles.current.clone().map(String::from);
    drop(profiles);
    let pipeline_id = new_job_id()?;
    let output_root = pipeline_directory(&request.batch.output_root, &pipeline_id)?;
    let original_profile = Config::profiles().await.latest_arc().current.clone();
    let original_profile_fingerprint = original_profile
        .as_ref()
        .and_then(|_| effective_runtime_fingerprint().ok());
    let proxies = handle::Handle::mihomo()
        .await
        .get_proxies()
        .await
        .map_err(|error| error.to_string())?;
    if let Some(active_profile) = active_profile.as_deref() {
        for candidate in request
            .candidates
            .iter()
            .filter(|candidate| candidate.profile_uid == active_profile)
        {
            let group = proxies.proxies.get(candidate.selection_group.as_str()).ok_or_else(|| {
                smartstring::alias::String::from(
                    "PIPELINE_SELECTOR_NOT_FOUND: selector is absent from the active runtime",
                )
            })?;
            if !group
                .all
                .as_ref()
                .is_some_and(|nodes| nodes.iter().any(|node| node == &candidate.requested_node))
            {
                return Err(
                    "PIPELINE_NODE_NOT_SELECTABLE: queued node is absent from the active runtime selector".into(),
                );
            }
        }
    }
    let mut restore_groups = std::collections::HashSet::new();
    let restore_selections = request
        .candidates
        .iter()
        .filter(|candidate| original_profile.as_deref() == Some(candidate.profile_uid.as_str()))
        .filter_map(|candidate| {
            let group = proxies.proxies.get(candidate.selection_group.as_str())?;
            if !restore_groups.insert(candidate.selection_group.clone()) {
                return None;
            }
            Some(PipelineSelection {
                group: candidate.selection_group.clone(),
                node: group.now.clone()?,
            })
        })
        .collect();
    let random_seed = match request.candidate_order_policy {
        PipelineCandidateOrderPolicy::BalancedSeeded => Some(request.random_seed.unwrap_or_else(unix_time_ms)),
        PipelineCandidateOrderPolicy::Fixed => None,
    };
    let schedule = PipelineSchedule::matrix(
        request.repetitions_per_candidate,
        request.candidates.len(),
        request.candidate_order_policy,
        random_seed,
    )
    .stringify_err()?;
    let planned_run_count = usize::from(request.repetitions_per_candidate)
        .checked_mul(request.batch.targets.len())
        .and_then(|value| value.checked_mul(request.candidates.len()))
        .ok_or_else(|| smartstring::alias::String::from("pipeline matrix size overflow"))?;
    let run_ids = (0..planned_run_count)
        .map(|_| new_job_id())
        .collect::<CmdResult<Vec<_>>>()?;
    let manifest = PipelineManifest::create_matrix(
        pipeline_id.clone(),
        output_root,
        PipelineConfigSnapshot {
            path: PathBuf::from(&request.batch.config_path),
            sha256: request.batch.config_sha256.clone(),
        },
        request.batch.targets.iter().map(pipeline_target).collect(),
        pipeline_execution_snapshot(&request.batch),
        request.candidates.clone(),
        run_ids,
        request.repetitions_per_candidate,
        PipelinePolicy {
            continue_on_run_failure: request.continue_on_run_failure,
            restore_original_state: true,
        },
        PipelineRestore {
            profile_uid: original_profile.map(Into::into),
            profile_fingerprint: original_profile_fingerprint,
            terminal_state: None,
            selections: restore_selections,
            checks: vec![],
            state: RestoreState::Pending,
            error: None,
        },
        schedule,
    )
    .stringify_err()?;

    CaptureLock::global()
        .acquire_owned(
            "pipeline",
            pipeline_id.clone(),
            "TrafficTracer profile and proxy pipeline is active",
        )
        .stringify_err()?;
    let environment = tt_get_environment_for_owner(
        app_handle.clone(),
        EnvironmentRequest {
            tun_interface: request.batch.tun_interface.clone(),
            physical_interface: request.batch.physical_interface.clone(),
            chrome_binary: request.batch.chrome_binary.clone(),
            output_root: request.batch.output_root.clone(),
            min_free_bytes: None,
        },
        Some(&pipeline_id),
    )
    .await;
    let environment = match environment {
        Ok(environment) => environment,
        Err(error) => {
            let _ = CaptureLock::global().release(&pipeline_id);
            return Err(error);
        }
    };
    if environment.level == CompleteEnvironmentLevel::Blocking {
        let _ = CaptureLock::global().release(&pipeline_id);
        return Err("TrafficTracer pipeline preflight has blocking diagnostics".into());
    }
    if let Err(error) = validate_pipeline_batch_contract(&manifest, &request.batch).await {
        let _ = CaptureLock::global().release(&pipeline_id);
        return Err(error);
    }
    if let Err(error) = manifest.persist() {
        let _ = CaptureLock::global().release(&pipeline_id);
        return Err(error.to_string().into());
    }
    let manifest_path = manifest.output_root.join(PIPELINE_MANIFEST_NAME);
    launch_pipeline_supervisor(app_handle, manifest_path, pipeline_id, request.batch);
    Ok(manifest)
}

fn pipeline_checkpoint(manifest: &mut PipelineManifest, stage: PipelineStage) -> Result<(), String> {
    manifest.checkpoint_run(stage).map_err(|error| error.to_string())?;
    manifest.persist().map_err(|error| error.to_string())?;
    Ok(())
}

async fn execute_pipeline_run(
    app_handle: &AppHandle,
    pipeline_id: &str,
    manifest: &mut PipelineManifest,
    index: usize,
    batch_template: &BatchStartRequest,
    interrupt: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<PipelineRunState, String> {
    let run = manifest.runs[index].clone();
    if let Some(state) = requested_pipeline_stop(interrupt, cancel) {
        return Ok(state);
    }
    pipeline_checkpoint(manifest, PipelineStage::ActivatingProfile)?;
    let active_profile_uid = Config::profiles().await.latest_arc().current.clone().map(String::from);
    let profile_already_active = active_profile_uid.as_deref() == Some(run.profile_uid.as_str());
    let previous_activation = manifest.runs[index]
        .evidence
        .as_ref()
        .and_then(|evidence| evidence.profile_activation.clone());
    let requested_at = previous_activation
        .as_ref()
        .map_or_else(Utc::now, |activation| activation.requested_at);
    let source_profile_uid = previous_activation
        .as_ref()
        .and_then(|activation| activation.source_profile_uid.clone())
        .or(active_profile_uid);
    let resumed_from_committed_state = run.resume_attempt > 0 && profile_already_active;
    manifest.runs[index]
        .evidence
        .get_or_insert_with(PipelineRunEvidence::default)
        .profile_activation = Some(PipelineProfileActivation {
        source_profile_uid,
        target_profile_uid: run.profile_uid.clone(),
        requested_at,
        profile_already_active,
        resumed_from_committed_state,
        profile_committed_at: previous_activation
            .as_ref()
            .and_then(|activation| activation.profile_committed_at),
        controller_verified_at: previous_activation
            .as_ref()
            .and_then(|activation| activation.controller_verified_at),
        last_completed_step: PipelineProfileActivationStep::ActivationRequested,
    });
    manifest.persist().map_err(|error| error.to_string())?;

    if !profile_already_active {
        let outcome = tokio::time::timeout(
            PIPELINE_PROFILE_ACTIVATION_TIMEOUT,
            super::profile::patch_profiles_config_for_owner(
                IProfiles {
                    current: Some(run.profile_uid.clone().into()),
                    items: None,
                },
                Some(pipeline_id),
            ),
        )
        .await
        .map_err(|_| {
            format!(
                "PROFILE_ACTIVATION_TIMEOUT: activating Profile {} exceeded {} seconds",
                run.profile_uid,
                PIPELINE_PROFILE_ACTIVATION_TIMEOUT.as_secs()
            )
        })?
        .map_err(|error| format!("PROFILE_ACTIVATION_REQUEST_FAILED: {error}"))?;
        if !outcome.is_valid() {
            return Err(format!("PROFILE_ACTIVATION_FAILED: {outcome}"));
        }
    }

    let committed_profile_uid = Config::profiles().await.latest_arc().current.clone().map(String::from);
    let committed_fingerprint = effective_runtime_fingerprint().ok();
    if committed_profile_uid.as_deref() != Some(run.profile_uid.as_str()) {
        return Err(format!(
            "PROFILE_COMMIT_READBACK_MISMATCH: expected Profile {}; observed Profile {}",
            run.profile_uid,
            committed_profile_uid.as_deref().unwrap_or("none")
        ));
    }
    if committed_fingerprint.as_deref() != Some(run.profile_fingerprint.as_str()) {
        return Err(format!(
            "CANDIDATE_CONFIG_DRIFT: Profile {} was bound to fingerprint {}; observed {} before repetition {}",
            run.profile_uid,
            run.profile_fingerprint,
            committed_fingerprint.as_deref().unwrap_or("unavailable"),
            run.repetition_index
        ));
    }
    if let Some(activation) = manifest.runs[index]
        .evidence
        .get_or_insert_with(PipelineRunEvidence::default)
        .profile_activation
        .as_mut()
    {
        activation.profile_committed_at.get_or_insert_with(Utc::now);
        activation.last_completed_step = PipelineProfileActivationStep::ProfileCommitted;
    }
    manifest.persist().map_err(|error| error.to_string())?;

    if let Some(state) = requested_pipeline_stop(interrupt, cancel) {
        return Ok(state);
    }
    pipeline_checkpoint(manifest, PipelineStage::WaitingController)?;
    wait_for_profile_controller(
        &run.profile_uid,
        Some(&run.profile_fingerprint),
        PIPELINE_CONTROLLER_TIMEOUT,
    )
    .await
    .map_err(|error| error.render())?;
    if let Some(activation) = manifest.runs[index]
        .evidence
        .get_or_insert_with(PipelineRunEvidence::default)
        .profile_activation
        .as_mut()
    {
        activation.controller_verified_at = Some(Utc::now());
        activation.last_completed_step = PipelineProfileActivationStep::ControllerVerified;
    }
    manifest.persist().map_err(|error| error.to_string())?;
    if let Some(state) = requested_pipeline_stop(interrupt, cancel) {
        return Ok(state);
    }

    pipeline_checkpoint(manifest, PipelineStage::SelectingProxy)?;
    let proxies = handle::Handle::mihomo()
        .await
        .get_proxies()
        .await
        .map_err(|error| format!("CONTROLLER_UNAVAILABLE: {error}"))?;
    let group = proxies
        .proxies
        .get(run.selection_group.as_str())
        .ok_or_else(|| "SELECTOR_NOT_FOUND: selector is absent from the active runtime".to_owned())?;
    if !group
        .all
        .as_ref()
        .is_some_and(|nodes| nodes.iter().any(|node| node.as_str() == run.requested_node))
    {
        return Err("NODE_NOT_SELECTABLE: requested node is absent from selector".into());
    }
    handle::Handle::mihomo()
        .await
        .select_node_for_group(&run.selection_group, &run.requested_node)
        .await
        .map_err(|error| format!("NODE_SELECTION_REQUEST_FAILED: {error}"))?;
    let selection_snapshot = wait_for_selected_snapshot(
        &run.profile_uid,
        &run.profile_fingerprint,
        &run.selection_group,
        &run.requested_node,
        PIPELINE_SELECTION_TIMEOUT,
    )
    .await
    .map_err(|error| error.render())?;
    manifest.runs[index].resolved_chain = selection_snapshot.resolved_chain.clone();
    manifest.runs[index].resolved_leaf = Some(selection_snapshot.resolved_leaf.clone());
    manifest.runs[index].expected_protocol = selection_snapshot.protocol.clone();
    manifest.runs[index]
        .evidence
        .get_or_insert_with(PipelineRunEvidence::default)
        .selection_snapshot = Some(selection_snapshot.clone());
    manifest.persist().map_err(|error| error.to_string())?;
    if let Some(state) = requested_pipeline_stop(interrupt, cancel) {
        return Ok(state);
    }

    pipeline_checkpoint(manifest, PipelineStage::DrainingConnections)?;
    let drain = observe_pipeline_connections().await;
    let drain_state = drain.state.clone();
    let drain_error = drain.error.clone();
    manifest.runs[index]
        .evidence
        .get_or_insert_with(PipelineRunEvidence::default)
        .drain = Some(drain);
    manifest.persist().map_err(|error| error.to_string())?;
    if drain_state != "preserved" {
        return Err(format!(
            "CONNECTION_OBSERVATION_FAILED: state={drain_state}; {}",
            drain_error.unwrap_or_else(|| "controller connection inventory was unavailable".into())
        ));
    }
    if let Some(state) = requested_pipeline_stop(interrupt, cancel) {
        return Ok(state);
    }

    pipeline_checkpoint(manifest, PipelineStage::Preflight)?;
    let mut batch = batch_template.clone();
    batch.output_root = run.output_path.to_string_lossy().into_owned();
    batch.options.proxy_selection_group = run.selection_group.clone();
    batch.options.expected_proxy_protocol.clear();
    if let Some(target_index) = run.target_index {
        batch.targets.retain(|target| target.index == target_index);
        if batch.targets.len() != 1 {
            return Err("PIPELINE_TARGET_SNAPSHOT_MISSING: matrix cell target is absent".into());
        }
        batch.options.analyze_after_capture = false;
        batch.application_retry.enabled = false;
        batch.application_retry.max_retries = 1;
    }

    let environment = tt_get_environment_for_owner(
        app_handle.clone(),
        EnvironmentRequest {
            tun_interface: batch.tun_interface.clone(),
            physical_interface: batch.physical_interface.clone(),
            chrome_binary: batch.chrome_binary.clone(),
            output_root: batch.output_root.clone(),
            min_free_bytes: None,
        },
        Some(pipeline_id),
    )
    .await
    .map_err(|error| error.to_string())?;
    if environment.level == CompleteEnvironmentLevel::Blocking {
        return Err("TrafficTracer environment has blocking diagnostics".into());
    }
    if let Some(state) = requested_pipeline_stop(interrupt, cancel) {
        return Ok(state);
    }
    let pre_batch_snapshot = wait_for_selected_snapshot(
        &run.profile_uid,
        &run.profile_fingerprint,
        &run.selection_group,
        &run.requested_node,
        PIPELINE_SELECTION_TIMEOUT,
    )
    .await
    .map_err(|error| error.render())?;
    let pre_batch_matches = proxy_snapshots_match(&selection_snapshot, &pre_batch_snapshot);
    manifest.runs[index]
        .evidence
        .get_or_insert_with(PipelineRunEvidence::default)
        .pre_batch_snapshot = Some(pre_batch_snapshot);
    manifest.persist().map_err(|error| error.to_string())?;
    if !pre_batch_matches {
        return Err("PRE_BATCH_NODE_DRIFT: selector chain changed after connection observation".into());
    }
    if let Some(state) = requested_pipeline_stop(interrupt, cancel) {
        return Ok(state);
    }
    let snapshot = if let Some(ref batch_id) = run.batch_id {
        tt_batch_resume_for_owner(batch_id.clone(), Some(pipeline_id)).await
    } else {
        tt_batch_start_for_owner(
            app_handle.clone(),
            batch,
            Some(pipeline_id),
            Some(PipelineOrchestration {
                pipeline_id: pipeline_id.to_owned(),
                run_id: run.run_id.clone(),
                run_ordinal: run.ordinal,
                repetition_index: run.target_index.map(|_| run.repetition_index),
                target_index: run.target_index,
                candidate_ordinal: run.target_index.map(|_| run.candidate_ordinal),
                candidate_position: manifest
                    .schedule
                    .repetition_candidate_orders
                    .get(usize::from(run.repetition_index.saturating_sub(1)))
                    .and_then(|order| order.iter().position(|ordinal| *ordinal == run.candidate_ordinal))
                    .map(|position| position + 1),
                application_retry_attempt: run.application_retry_attempt,
                profile_uid: run.profile_uid.clone(),
                selection_group: run.selection_group.clone(),
                requested_node: run.requested_node.clone(),
            }),
        )
        .await
    }
    .map_err(|error| error.to_string())?;
    manifest.runs[index].batch_id = Some(snapshot.job_id.clone());
    pipeline_checkpoint(manifest, PipelineStage::StartingBatch)?;
    let batch_id = snapshot.job_id;
    let startup_started = tokio::time::Instant::now();
    let mut batch_confirmed = false;
    let mut stop_requested = false;
    let mut reconciliation_started = false;
    let mut status_error_started: Option<tokio::time::Instant> = None;
    loop {
        if !stop_requested && let Some(requested) = requested_pipeline_stop(interrupt, cancel) {
            let (batch_result, fallback_method, reason) = match requested {
                PipelineRunState::Cancelled => (
                    tt_batch_cancel(batch_id.clone(), Some("Pipeline cancelled by user".into())).await,
                    RequestMethod::JobCancel,
                    "Pipeline cancelled before Batch status became visible",
                ),
                PipelineRunState::Interrupted => (
                    tt_batch_interrupt(batch_id.clone(), Some("Pipeline interrupted by user".into())).await,
                    RequestMethod::JobInterrupt,
                    "Pipeline interrupted before Batch status became visible",
                ),
                _ => unreachable!("pipeline stop request is terminal"),
            };
            if batch_result.is_err()
                && let Ok(client) = WorkerManager::global().client()
            {
                let _ = client
                    .request::<_, JobSnapshot>(
                        fallback_method,
                        CancelJobParams {
                            job_id: batch_id.clone(),
                            reason: reason.into(),
                        },
                    )
                    .await;
            }
            stop_requested = true;
        }

        let status = match tt_batch_status(batch_id.clone()).await {
            Ok(status) => {
                status_error_started = None;
                status
            }
            Err(batch_error) => {
                let error_started = *status_error_started.get_or_insert_with(tokio::time::Instant::now);
                let manager = WorkerManager::global();
                let job_status = match manager.client() {
                    Ok(client) => client
                        .request::<_, JobSnapshot>(
                            RequestMethod::JobStatus,
                            JobIdParams {
                                job_id: batch_id.clone(),
                            },
                        )
                        .await
                        .map_err(|error| error.to_string()),
                    Err(error) => Err(error.to_string()),
                };
                match job_status {
                    Ok(job) if job.state.terminal() => {
                        let _ = manager.mark_ready(&batch_id);
                        return Err(format!("BATCH_STATUS_UNAVAILABLE_AFTER_JOB_TERMINAL: {batch_error}"));
                    }
                    Ok(_) => {
                        if !reconciliation_started && error_started.elapsed() >= Duration::from_secs(15) {
                            pipeline_checkpoint(manifest, PipelineStage::ReconcilingBatch)?;
                            if let Ok(client) = manager.client() {
                                let _ = client
                                    .request::<_, JobSnapshot>(
                                        RequestMethod::JobCancel,
                                        CancelJobParams {
                                            job_id: batch_id.clone(),
                                            reason: "Batch manifest did not become visible during startup".into(),
                                        },
                                    )
                                    .await;
                            }
                            reconciliation_started = true;
                            stop_requested = true;
                        }
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        continue;
                    }
                    Err(_job_error) if manager.state() == WorkerManagerState::Busy => {
                        if !reconciliation_started {
                            pipeline_checkpoint(manifest, PipelineStage::ReconcilingBatch)?;
                            reconciliation_started = true;
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        continue;
                    }
                    Err(job_error) => {
                        return Err(format!(
                            "BATCH_STATUS_RECONCILIATION_FAILED: {batch_error}; job status: {job_error}"
                        ));
                    }
                }
            }
        };

        let batch_visible = status.get("batch").is_some_and(|batch| !batch.is_null());
        if !batch_visible {
            let job_terminal = status
                .pointer("/job/state")
                .and_then(Value::as_str)
                .is_some_and(terminal_batch_state);
            if job_terminal {
                let _ = WorkerManager::global().mark_ready(&batch_id);
                return Err(
                    "BATCH_MANIFEST_UNAVAILABLE_AFTER_JOB_TERMINAL: the Worker Job ended before its Batch manifest became visible"
                        .into(),
                );
            }
            if WorkerManager::global().state() != WorkerManagerState::Busy {
                return Err("BATCH_MANIFEST_UNAVAILABLE: no active Worker Job owns the requested Batch".into());
            }
            if !reconciliation_started && startup_started.elapsed() >= Duration::from_secs(15) {
                pipeline_checkpoint(manifest, PipelineStage::ReconcilingBatch)?;
                if let Ok(client) = WorkerManager::global().client() {
                    let _ = client
                        .request::<_, JobSnapshot>(
                            RequestMethod::JobCancel,
                            CancelJobParams {
                                job_id: batch_id.clone(),
                                reason: "Batch manifest did not become visible during startup".into(),
                            },
                        )
                        .await;
                }
                reconciliation_started = true;
                stop_requested = true;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }
        if !batch_confirmed {
            pipeline_checkpoint(manifest, PipelineStage::RunningBatch)?;
            batch_confirmed = true;
        }
        if let Some(state) = status
            .pointer("/batch/state")
            .and_then(Value::as_str)
            .filter(|state| terminal_batch_state(state))
        {
            if !batch_status_can_release_capture(&status) {
                if manifest.stage != PipelineStage::FinalizingBatch {
                    pipeline_checkpoint(manifest, PipelineStage::FinalizingBatch)?;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
            pipeline_checkpoint(manifest, PipelineStage::VerifyingProtocol)?;
            let observed = observed_run_evidence(&run.output_path);
            manifest.runs[index].observed_protocol = observed_run_protocol(&observed);
            let end_result = wait_for_end_snapshot(
                &run.selection_group,
                &run.profile_uid,
                &run.profile_fingerprint,
                PIPELINE_END_SNAPSHOT_TIMEOUT,
            )
            .await;
            let (end_snapshot, end_error) = match end_result {
                Ok(snapshot) => (Some(snapshot), None),
                Err(error) => (None, Some(error)),
            };
            let start_snapshot = manifest.runs[index]
                .evidence
                .as_ref()
                .and_then(|evidence| {
                    evidence
                        .pre_batch_snapshot
                        .as_ref()
                        .or(evidence.selection_snapshot.as_ref())
                })
                .cloned()
                .ok_or_else(|| {
                    "PIPELINE_START_SNAPSHOT_MISSING: pre-Batch node evidence was not persisted".to_owned()
                })?;
            let verification =
                classify_run_verification(&start_snapshot, end_snapshot.as_ref(), &observed, end_error.as_ref());
            let verification_requires_attention = verification_requires_attention(&verification);
            let evidence = manifest.runs[index]
                .evidence
                .get_or_insert_with(PipelineRunEvidence::default);
            evidence.end_snapshot = end_snapshot;
            evidence.verification = Some(verification);
            let effective_sessions = manifest.runs[index]
                .batch_id
                .as_deref()
                .and_then(|batch_id| batch_effective_sessions(&run.output_path, batch_id));
            if run.target_index.is_some() {
                return Ok(match state {
                    "completed" if effective_sessions.is_some() => PipelineRunState::Captured,
                    "completed" => {
                        return Err(
                            "PIPELINE_CAPTURE_SESSION_MISSING: capture completed without an effective Session".into(),
                        );
                    }
                    "cancelled" => PipelineRunState::Cancelled,
                    "interrupted" => PipelineRunState::Interrupted,
                    _ => PipelineRunState::Failed,
                });
            }
            let quality = pipeline_run_quality(&run.output_path, effective_sessions.as_ref());
            let quality_requires_attention = run_quality_requires_attention(&quality);
            manifest.runs[index].quality = Some(quality);
            return Ok(match state {
                "completed" if quality_requires_attention || verification_requires_attention => {
                    PipelineRunState::Degraded
                }
                "completed" => PipelineRunState::Completed,
                "cancelled" => PipelineRunState::Cancelled,
                "interrupted" => PipelineRunState::Interrupted,
                _ => PipelineRunState::Failed,
            });
        }
        tokio::time::sleep(if batch_confirmed {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(250)
        })
        .await;
    }
}

fn persist_restore_check(manifest: &mut PipelineManifest, check: PipelineRestoreCheck) {
    manifest.restore.checks.push(check);
    manifest.updated_at = Utc::now();
    let _ = manifest.persist();
}

fn restore_check(
    component: &str,
    target: &str,
    requested: &str,
    observed: Option<String>,
    state: &str,
    error: Option<&PipelineBarrierError>,
) -> PipelineRestoreCheck {
    PipelineRestoreCheck {
        component: component.into(),
        target: target.into(),
        requested: requested.into(),
        observed,
        state: state.into(),
        code: error.map(|value| value.code.to_owned()),
        message: error.map(|value| value.message.clone()),
        checked_at: Utc::now(),
    }
}

async fn restore_pipeline(manifest: &mut PipelineManifest, pipeline_id: &str) {
    let terminal_state = if manifest.state == PipelineState::RestoreFailed {
        manifest
            .restore
            .terminal_state
            .unwrap_or(PipelineState::CompletedWithErrors)
    } else {
        manifest.state
    };
    manifest.restore.terminal_state = Some(terminal_state);
    manifest.state = PipelineState::Restoring;
    manifest.stage = PipelineStage::Restoring;
    manifest.restore.state = RestoreState::Restoring;
    manifest.restore.error = None;
    manifest.restore.checks.clear();
    manifest.updated_at = Utc::now();
    let _ = manifest.persist();
    let mut failure: Option<PipelineBarrierError> = None;
    let profile_uid = manifest.restore.profile_uid.clone();
    let mut effective_restore_fingerprint = manifest.restore.profile_fingerprint.clone();

    if let Some(profile_uid) = profile_uid.as_deref() {
        if Config::profiles().await.latest_arc().current.as_deref() != Some(profile_uid) {
            let request = super::profile::patch_profiles_config_for_owner(
                IProfiles {
                    current: Some(profile_uid.into()),
                    items: None,
                },
                Some(pipeline_id),
            )
            .await;
            let request_error = match request {
                Ok(outcome) if outcome.is_valid() => None,
                Ok(outcome) => Some(PipelineBarrierError::new(
                    "RESTORE_PROFILE_REQUEST_FAILED",
                    outcome.to_string(),
                )),
                Err(error) => Some(PipelineBarrierError::new(
                    "RESTORE_PROFILE_REQUEST_FAILED",
                    error.to_string(),
                )),
            };
            if let Some(error) = request_error {
                persist_restore_check(
                    manifest,
                    restore_check(
                        "profile",
                        profile_uid,
                        profile_uid,
                        None,
                        "request_failed",
                        Some(&error),
                    ),
                );
                failure = Some(error);
            }
        }

        if failure.is_none() {
            match wait_for_profile_controller(
                profile_uid,
                effective_restore_fingerprint.as_deref(),
                PIPELINE_CONTROLLER_TIMEOUT,
            )
            .await
            {
                Ok(()) => {
                    let observed = Config::profiles().await.latest_arc().current.clone().map(String::from);
                    effective_restore_fingerprint.get_or_insert(effective_runtime_fingerprint().unwrap_or_default());
                    persist_restore_check(
                        manifest,
                        restore_check("profile", profile_uid, profile_uid, observed, "passed", None),
                    );
                }
                Err(error) => {
                    let state = if error.code == "CONTROLLER_UNAVAILABLE" {
                        "controller_unavailable"
                    } else {
                        "readback_mismatch"
                    };
                    let observed = Config::profiles().await.latest_arc().current.clone().map(String::from);
                    let wrapped = PipelineBarrierError::new(
                        match state {
                            "controller_unavailable" => "RESTORE_CONTROLLER_UNAVAILABLE",
                            _ => "RESTORE_PROFILE_READBACK_MISMATCH",
                        },
                        error.render(),
                    );
                    persist_restore_check(
                        manifest,
                        restore_check("profile", profile_uid, profile_uid, observed, state, Some(&wrapped)),
                    );
                    failure = Some(wrapped);
                }
            }
        }
    }

    if failure.is_none()
        && let Some(profile_uid) = profile_uid.as_deref()
    {
        let fingerprint = effective_restore_fingerprint.unwrap_or_default();
        for selection in manifest.restore.selections.clone() {
            let request = handle::Handle::mihomo()
                .await
                .select_node_for_group(&selection.group, &selection.node)
                .await;
            if let Err(error) = request {
                let wrapped = PipelineBarrierError::new("RESTORE_SELECTOR_REQUEST_FAILED", error.to_string());
                persist_restore_check(
                    manifest,
                    restore_check(
                        "selector",
                        &selection.group,
                        &selection.node,
                        None,
                        "request_failed",
                        Some(&wrapped),
                    ),
                );
                failure.get_or_insert(wrapped);
                continue;
            }
            match wait_for_selected_snapshot(
                profile_uid,
                &fingerprint,
                &selection.group,
                &selection.node,
                PIPELINE_SELECTION_TIMEOUT,
            )
            .await
            {
                Ok(snapshot) => persist_restore_check(
                    manifest,
                    restore_check(
                        "selector",
                        &selection.group,
                        &selection.node,
                        Some(snapshot.selected_node),
                        "passed",
                        None,
                    ),
                ),
                Err(error) => {
                    let state = if error.code == "CONTROLLER_UNAVAILABLE" {
                        "controller_unavailable"
                    } else {
                        "readback_mismatch"
                    };
                    let wrapped = PipelineBarrierError::new(
                        match state {
                            "controller_unavailable" => "RESTORE_CONTROLLER_UNAVAILABLE",
                            _ => "RESTORE_SELECTOR_READBACK_MISMATCH",
                        },
                        error.render(),
                    );
                    persist_restore_check(
                        manifest,
                        restore_check(
                            "selector",
                            &selection.group,
                            &selection.node,
                            None,
                            state,
                            Some(&wrapped),
                        ),
                    );
                    failure.get_or_insert(wrapped);
                }
            }
        }
    }

    let result: Result<(), PipelineBarrierError> = failure.map_or(Ok(()), Err);
    match result {
        Ok(()) => {
            manifest.restore.state = RestoreState::Restored;
            manifest.state = terminal_state;
            manifest.stage = PipelineStage::Finished;
        }
        Err(error) => {
            manifest.restore.state = RestoreState::Failed;
            manifest.restore.error = Some(PipelineError {
                code: error.code.into(),
                message: error.message,
            });
            manifest.state = PipelineState::RestoreFailed;
        }
    }
    manifest.updated_at = Utc::now();
    let _ = manifest.persist();
}

async fn materialize_pipeline_candidate(
    manifest: &mut PipelineManifest,
    pipeline_id: &str,
    candidate_ordinal: u16,
) -> Result<(), String> {
    let candidate = manifest
        .runs
        .iter()
        .find(|run| run.candidate_ordinal == candidate_ordinal)
        .cloned()
        .ok_or_else(|| "PIPELINE_CANDIDATE_NOT_FOUND: candidate has no runs".to_owned())?;

    if Config::profiles().await.latest_arc().current.as_deref() != Some(candidate.profile_uid.as_str()) {
        let outcome = tokio::time::timeout(
            PIPELINE_PROFILE_ACTIVATION_TIMEOUT,
            super::profile::patch_profiles_config_for_owner(
                IProfiles {
                    current: Some(candidate.profile_uid.clone().into()),
                    items: None,
                },
                Some(pipeline_id),
            ),
        )
        .await
        .map_err(|_| {
            format!(
                "PROFILE_ACTIVATION_TIMEOUT: materializing Profile {} exceeded {} seconds",
                candidate.profile_uid,
                PIPELINE_PROFILE_ACTIVATION_TIMEOUT.as_secs()
            )
        })?
        .map_err(|error| format!("PROFILE_ACTIVATION_REQUEST_FAILED: {error}"))?;
        if !outcome.is_valid() {
            return Err(format!("PROFILE_ACTIVATION_FAILED: {outcome}"));
        }
    }

    wait_for_profile_controller(&candidate.profile_uid, None, PIPELINE_CONTROLLER_TIMEOUT)
        .await
        .map_err(|error| error.render())?;
    let fingerprint =
        effective_runtime_fingerprint().map_err(|error| format!("PROFILE_FINGERPRINT_UNAVAILABLE: {error}"))?;
    handle::Handle::mihomo()
        .await
        .select_node_for_group(&candidate.selection_group, &candidate.requested_node)
        .await
        .map_err(|error| format!("SELECTOR_REQUEST_FAILED: {error}"))?;
    let snapshot = wait_for_selected_snapshot(
        &candidate.profile_uid,
        &fingerprint,
        &candidate.selection_group,
        &candidate.requested_node,
        PIPELINE_SELECTION_TIMEOUT,
    )
    .await
    .map_err(|error| error.render())?;
    let bound_at = Utc::now();
    let changed = manifest
        .bind_candidate_profile(candidate_ordinal, fingerprint, bound_at)
        .map_err(|error| error.to_string())?;
    for run in manifest
        .runs
        .iter_mut()
        .filter(|run| run.candidate_ordinal == candidate_ordinal)
    {
        run.resolved_chain = snapshot.resolved_chain.clone();
        run.resolved_leaf = Some(snapshot.resolved_leaf.clone());
        run.expected_protocol = snapshot.protocol.clone();
    }
    logging!(
        info,
        Type::System,
        "TrafficTracer candidate materialized; pipeline={pipeline_id}; candidate={candidate_ordinal}; profile={}; node={}; protocol={}; snapshot_changed={changed}",
        candidate.profile_uid,
        candidate.requested_node,
        snapshot.protocol
    );
    manifest.persist().map_err(|error| error.to_string())?;
    Ok(())
}

async fn materialize_pipeline_candidates(manifest: &mut PipelineManifest, pipeline_id: &str) -> Result<(), String> {
    let candidates = manifest
        .runs
        .iter()
        .filter(|run| {
            matches!(run.state, PipelineRunState::Pending | PipelineRunState::Interrupted)
                && run.profile_bound_at.is_none()
        })
        .map(|run| run.candidate_ordinal)
        .collect::<std::collections::BTreeSet<_>>();
    if candidates.is_empty() {
        return Ok(());
    }
    manifest.state = PipelineState::Validating;
    manifest.stage = PipelineStage::Materializing;
    manifest.updated_at = Utc::now();
    manifest.persist().map_err(|error| error.to_string())?;

    for candidate_ordinal in candidates {
        if let Err(message) = materialize_pipeline_candidate(manifest, pipeline_id, candidate_ordinal).await {
            let error = pipeline_run_error(message);
            manifest
                .fail_candidate_materialization(candidate_ordinal, error.clone())
                .map_err(|failure| failure.to_string())?;
            manifest.persist().map_err(|failure| failure.to_string())?;
            if !manifest.policy.continue_on_run_failure {
                manifest.state = PipelineState::Failed;
                manifest.stage = PipelineStage::Finished;
                manifest.updated_at = Utc::now();
                manifest.persist().map_err(|failure| failure.to_string())?;
                return Err(error.message);
            }
        }
    }
    manifest.state = PipelineState::Running;
    manifest.stage = PipelineStage::Queued;
    manifest.updated_at = Utc::now();
    manifest.persist().map_err(|error| error.to_string())?;
    Ok(())
}

fn non_retryable_candidate_error(code: &str) -> bool {
    matches!(
        code,
        "CANDIDATE_CONFIG_DRIFT"
            | "PROFILE_COMMIT_READBACK_MISMATCH"
            | "PROFILE_READBACK_MISMATCH"
            | "SELECTOR_NOT_FOUND"
            | "PROXY_LEAF_NOT_FOUND"
            | "PROXY_CHAIN_CYCLE"
            | "PROXY_CHAIN_TOO_DEEP"
            | "NODE_READBACK_MISMATCH"
    )
}

fn systemic_pipeline_error(error: &PipelineError) -> bool {
    if matches!(
        error.code.as_str(),
        "PIPELINE_BATCH_PREFLIGHT_FAILED" | "PROTOCOL_VERSION_MISMATCH" | "METHOD_NOT_FOUND"
    ) {
        return true;
    }
    let message = error.message.to_ascii_uppercase();
    [
        "CONTRACT_VALIDATION_FAILED",
        "WORKER RETURNED INVALIDPARAMS",
        "PROTOCOL_VERSION_MISMATCH",
        "METHOD_NOT_FOUND",
        "WORKER METHOD IS NOT SUPPORTED",
        "FROZEN SCHEDULE",
        "MATRIX RUN ORDER DOES NOT MATCH",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

async fn execute_pipeline_analysis(
    app_handle: &AppHandle,
    pipeline_id: &str,
    manifest: &mut PipelineManifest,
    index: usize,
    batch_template: &BatchStartRequest,
    interrupt: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<PipelineRunState, String> {
    let run = manifest.runs[index].clone();
    if run.session_ids.is_empty() {
        return Err("PIPELINE_ANALYSIS_SESSION_MISSING: captured cell has no Session identity".into());
    }
    let environment = tt_get_environment_for_owner(
        app_handle.clone(),
        EnvironmentRequest {
            tun_interface: batch_template.tun_interface.clone(),
            physical_interface: batch_template.physical_interface.clone(),
            chrome_binary: batch_template.chrome_binary.clone(),
            output_root: run.output_path.to_string_lossy().into_owned(),
            min_free_bytes: None,
        },
        Some(pipeline_id),
    )
    .await
    .map_err(|error| error.to_string())?;
    if environment.level == CompleteEnvironmentLevel::Blocking {
        return Err("TrafficTracer deferred-analysis environment has blocking diagnostics".into());
    }

    for session_id in &run.session_ids {
        if let Some(requested) = requested_pipeline_stop(interrupt, cancel) {
            return Ok(requested);
        }
        let snapshot = tt_analysis_start(
            session_id.clone(),
            Some(AnalysisOptions {
                split_pcaps: batch_template.options.capture_packets
                    && batch_template.options.pcap_split_mode == "unique_connections",
                pcap_split_mode: if batch_template.options.capture_packets {
                    batch_template.options.pcap_split_mode.clone()
                } else {
                    "none".into()
                },
                write_flow_index: true,
                overwrite: true,
            }),
        )
        .await
        .map_err(|error| error.to_string())?;
        manifest.runs[index].analysis_job_id = Some(snapshot.job_id.clone());
        manifest.runs[index].stage = PipelineStage::AnalysisWave;
        manifest.stage = PipelineStage::AnalysisWave;
        manifest.updated_at = Utc::now();
        manifest.persist().map_err(|error| error.to_string())?;

        let job_id = snapshot.job_id;
        let mut stop_requested = false;
        loop {
            if !stop_requested {
                if cancel.load(Ordering::Acquire) {
                    let _ = tt_capture_cancel(job_id.clone(), Some("Pipeline cancelled during analysis".into())).await;
                    stop_requested = true;
                } else if interrupt.load(Ordering::Acquire) {
                    if let Ok(client) = WorkerManager::global().client() {
                        let _ = client
                            .request::<_, JobSnapshot>(
                                RequestMethod::JobInterrupt,
                                CancelJobParams {
                                    job_id: job_id.clone(),
                                    reason: "Pipeline interrupted during analysis".into(),
                                },
                            )
                            .await;
                    }
                    stop_requested = true;
                }
            }
            let status = tt_capture_get(job_id.clone())
                .await
                .map_err(|error| format!("PIPELINE_ANALYSIS_STATUS_FAILED: {error}"))?;
            if status.state.terminal() {
                match status.state {
                    JobState::Completed => break,
                    JobState::Cancelled => return Ok(PipelineRunState::Cancelled),
                    JobState::Interrupted => return Ok(PipelineRunState::Interrupted),
                    JobState::Failed => {
                        return Err(format!(
                            "PIPELINE_ANALYSIS_FAILED: {}",
                            status.error.map_or_else(|| status.message, |error| error.to_string(),)
                        ));
                    }
                    _ => unreachable!("terminal analysis state"),
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    let effective_sessions = run.session_ids.iter().cloned().collect::<HashSet<_>>();
    let quality = pipeline_run_quality(&run.output_path, Some(&effective_sessions));
    let degraded = run_quality_requires_attention(&quality)
        || manifest.runs[index]
            .evidence
            .as_ref()
            .and_then(|evidence| evidence.verification.as_ref())
            .is_some_and(verification_requires_attention);
    manifest.runs[index].quality = Some(quality);
    Ok(if degraded {
        PipelineRunState::Degraded
    } else {
        PipelineRunState::Completed
    })
}

fn matrix_application_retry_required(run: &crate::core::traffic_tracer::pipeline::PipelineRun) -> bool {
    const REASONS: &[&str] = &[
        "CRITICAL_RESOURCE_FAILURE_BURST",
        "MAIN_DOCUMENT_NETWORK_ERROR",
        "MAIN_DOCUMENT_NOT_OBSERVED",
        "MAIN_DOCUMENT_RESPONSE_UNKNOWN",
        "MAIN_DOCUMENT_SERVER_ERROR",
        "MAIN_DOCUMENT_TRANSIENT_HTTP_ERROR",
        "NAVIGATION_COMPLETION_UNCERTAIN",
        "PLAYBACK_STATE_UNKNOWN",
        "PLAYER_NOT_CREATED",
        "VIDEO_ELEMENT_NOT_CREATED",
        "MEDIA_NOT_READY",
        "MEDIA_NOT_ADVANCING",
        "PRIMARY_CONTENT_NOT_OBSERVED",
    ];
    run.quality.as_ref().is_some_and(|quality| {
        quality.application_issues.iter().any(|issue| {
            let eligible_state = matches!(issue.state.as_str(), "failed" | "indeterminate")
                || (issue.state == "degraded" && issue.reason.as_deref() == Some("CRITICAL_RESOURCE_FAILURE_BURST"));
            eligible_state && issue.reason.as_deref().is_some_and(|reason| REASONS.contains(&reason))
        })
    })
}

async fn run_matrix_pipeline(
    app_handle: &AppHandle,
    pipeline_id: &str,
    manifest: &mut PipelineManifest,
    batch: &BatchStartRequest,
    interrupt: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    for repetition_index in 1..=manifest.repetitions_per_candidate {
        loop {
            loop {
                if cancel.load(Ordering::Acquire) || interrupt.load(Ordering::Acquire) {
                    manifest.state = if cancel.load(Ordering::Acquire) {
                        PipelineState::Cancelled
                    } else {
                        PipelineState::Interrupted
                    };
                    manifest.stage = PipelineStage::Finished;
                    manifest.updated_at = Utc::now();
                    manifest.persist().map_err(|error| error.to_string())?;
                    return Ok(());
                }
                let Some(index) = manifest
                    .begin_next_capture(repetition_index)
                    .map_err(|error| error.to_string())?
                else {
                    break;
                };
                manifest.persist().map_err(|error| error.to_string())?;
                match execute_pipeline_run(app_handle, pipeline_id, manifest, index, batch, interrupt, cancel).await {
                    Ok(PipelineRunState::Captured) => {
                        let mut session_ids = manifest.runs[index]
                            .batch_id
                            .as_deref()
                            .and_then(|batch_id| batch_effective_sessions(&manifest.runs[index].output_path, batch_id))
                            .unwrap_or_default()
                            .into_iter()
                            .collect::<Vec<_>>();
                        session_ids.sort();
                        manifest
                            .finish_capture(session_ids)
                            .map_err(|error| error.to_string())?;
                    }
                    Ok(PipelineRunState::Interrupted) => {
                        manifest
                            .finish_run(PipelineRunState::Interrupted, None)
                            .map_err(|error| error.to_string())?;
                        manifest.state = PipelineState::Interrupted;
                    }
                    Ok(PipelineRunState::Cancelled) => {
                        manifest
                            .finish_run(PipelineRunState::Cancelled, None)
                            .map_err(|error| error.to_string())?;
                        manifest.state = PipelineState::Cancelled;
                    }
                    Ok(state) => {
                        manifest.finish_run(state, None).map_err(|error| error.to_string())?;
                    }
                    Err(message) => {
                        let candidate_ordinal = manifest.runs[index].candidate_ordinal;
                        let error = pipeline_run_error(message);
                        let stop_candidate = non_retryable_candidate_error(&error.code);
                        let systemic = systemic_pipeline_error(&error);
                        manifest
                            .finish_run(PipelineRunState::Failed, Some(error.clone()))
                            .map_err(|failure| failure.to_string())?;
                        if systemic {
                            manifest.skip_remaining_runs(&error);
                            manifest.state = PipelineState::Failed;
                        } else if stop_candidate {
                            manifest.skip_remaining_candidate_runs(candidate_ordinal, &error);
                        }
                        if !systemic && !manifest.policy.continue_on_run_failure {
                            manifest.state = PipelineState::Failed;
                        }
                    }
                }
                manifest.persist().map_err(|error| error.to_string())?;
                if matches!(
                    manifest.state,
                    PipelineState::Interrupted | PipelineState::Cancelled | PipelineState::Failed
                ) {
                    return Ok(());
                }
            }

            manifest.stage = PipelineStage::AnalysisWave;
            manifest.updated_at = Utc::now();
            manifest.persist().map_err(|error| error.to_string())?;
            loop {
                if cancel.load(Ordering::Acquire) || interrupt.load(Ordering::Acquire) {
                    manifest.state = if cancel.load(Ordering::Acquire) {
                        PipelineState::Cancelled
                    } else {
                        PipelineState::Interrupted
                    };
                    manifest.stage = PipelineStage::Finished;
                    manifest.updated_at = Utc::now();
                    manifest.persist().map_err(|error| error.to_string())?;
                    return Ok(());
                }
                let Some(index) = manifest
                    .begin_next_analysis(repetition_index)
                    .map_err(|error| error.to_string())?
                else {
                    break;
                };
                manifest.persist().map_err(|error| error.to_string())?;
                match execute_pipeline_analysis(app_handle, pipeline_id, manifest, index, batch, interrupt, cancel)
                    .await
                {
                    Ok(PipelineRunState::Interrupted) => {
                        manifest.runs[index].state = PipelineRunState::Captured;
                        manifest.runs[index].stage = PipelineStage::Checkpoint;
                        manifest.current_run_index = None;
                        manifest.state = PipelineState::Interrupted;
                        manifest.stage = PipelineStage::Finished;
                        manifest.updated_at = Utc::now();
                    }
                    Ok(PipelineRunState::Cancelled) => {
                        manifest
                            .finish_analysis(PipelineRunState::Cancelled, None)
                            .map_err(|error| error.to_string())?;
                        manifest.state = PipelineState::Cancelled;
                    }
                    Ok(state)
                        if matches!(state, PipelineRunState::Completed | PipelineRunState::Degraded)
                            && batch.application_retry.enabled
                            && manifest.runs[index].application_retry_attempt < batch.application_retry.max_retries
                            && matrix_application_retry_required(&manifest.runs[index]) =>
                    {
                        manifest
                            .schedule_application_retry(batch.application_retry.max_retries)
                            .map_err(|error| error.to_string())?;
                    }
                    Ok(state) => {
                        manifest
                            .finish_analysis(state, None)
                            .map_err(|error| error.to_string())?;
                    }
                    Err(message) => {
                        let error = pipeline_run_error(message);
                        let systemic = systemic_pipeline_error(&error);
                        manifest
                            .finish_analysis(PipelineRunState::Failed, Some(error.clone()))
                            .map_err(|failure| failure.to_string())?;
                        if systemic {
                            manifest.skip_remaining_runs(&error);
                            manifest.state = PipelineState::Failed;
                        } else if !manifest.policy.continue_on_run_failure {
                            manifest.state = PipelineState::Failed;
                        }
                    }
                }
                manifest.persist().map_err(|error| error.to_string())?;
                if matches!(
                    manifest.state,
                    PipelineState::Interrupted | PipelineState::Cancelled | PipelineState::Failed
                ) {
                    return Ok(());
                }
            }
            if !manifest
                .runs
                .iter()
                .any(|run| run.repetition_index == repetition_index && run.state == PipelineRunState::RetryPending)
            {
                break;
            }
        }
    }
    manifest.finalize_matrix().map_err(|error| error.to_string())?;
    manifest.persist().map_err(|error| error.to_string())?;
    Ok(())
}

async fn run_pipeline(
    app_handle: AppHandle,
    manifest_path: PathBuf,
    batch: BatchStartRequest,
    interrupt: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut manifest = PipelineManifest::load(&manifest_path).map_err(|error| error.to_string())?;
    let pipeline_id = manifest.pipeline_id.clone();
    materialize_pipeline_candidates(&mut manifest, &pipeline_id).await?;
    if manifest.schedule.mode == PipelineScheduleMode::RepetitionTargetCandidate {
        run_matrix_pipeline(&app_handle, &pipeline_id, &mut manifest, &batch, &interrupt, &cancel).await?;
        restore_pipeline(&mut manifest, &pipeline_id).await;
        return Ok(());
    }

    loop {
        if cancel.load(Ordering::Acquire) || interrupt.load(Ordering::Acquire) {
            manifest.state = if cancel.load(Ordering::Acquire) {
                PipelineState::Cancelled
            } else {
                PipelineState::Interrupted
            };
            manifest.stage = PipelineStage::Finished;
            manifest.updated_at = Utc::now();
            manifest.persist().map_err(|error| error.to_string())?;
            break;
        }
        let Some(index) = manifest.begin_next_run().map_err(|error| error.to_string())? else {
            break;
        };
        manifest.persist().map_err(|error| error.to_string())?;
        match execute_pipeline_run(
            &app_handle,
            &pipeline_id,
            &mut manifest,
            index,
            &batch,
            &interrupt,
            &cancel,
        )
        .await
        {
            Ok(PipelineRunState::Interrupted) => {
                manifest
                    .finish_run(PipelineRunState::Interrupted, None)
                    .map_err(|error| error.to_string())?;
                manifest.state = PipelineState::Interrupted;
            }
            Ok(PipelineRunState::Cancelled) => {
                manifest
                    .finish_run(PipelineRunState::Cancelled, None)
                    .map_err(|error| error.to_string())?;
                manifest.state = PipelineState::Cancelled;
            }
            Ok(state) => {
                manifest.finish_run(state, None).map_err(|error| error.to_string())?;
            }
            Err(message) => {
                let candidate_ordinal = manifest.runs[index].candidate_ordinal;
                let error = pipeline_run_error(message);
                let stop_candidate = non_retryable_candidate_error(&error.code);
                let systemic = systemic_pipeline_error(&error);
                manifest
                    .finish_run(PipelineRunState::Failed, Some(error.clone()))
                    .map_err(|failure| failure.to_string())?;
                if systemic {
                    manifest.skip_remaining_runs(&error);
                    manifest.state = PipelineState::Failed;
                } else if stop_candidate {
                    manifest.skip_remaining_candidate_runs(candidate_ordinal, &error);
                }
                if !systemic && !manifest.policy.continue_on_run_failure {
                    manifest.state = PipelineState::Failed;
                }
            }
        }
        manifest.persist().map_err(|error| error.to_string())?;
        if matches!(
            manifest.state,
            PipelineState::Interrupted | PipelineState::Cancelled | PipelineState::Failed
        ) {
            break;
        }
    }
    if manifest.state == PipelineState::Running {
        let _ = manifest.begin_next_run().map_err(|error| error.to_string())?;
    }
    restore_pipeline(&mut manifest, &pipeline_id).await;
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct PipelineListEntry {
    pub pipeline_id: String,
    pub output_root: PathBuf,
    pub state: PipelineState,
    pub updated_at: DateTime<Utc>,
    pub completed_runs: usize,
    pub total_runs: usize,
    pub candidate_count: usize,
    pub repetitions_per_candidate: u16,
}

fn active_pipeline_matches(pipeline_id: &str) -> bool {
    pipeline_runtime()
        .active
        .lock()
        .as_ref()
        .is_some_and(|active| active.pipeline_id == pipeline_id)
}

#[tauri::command]
pub fn tt_pipeline_status(pipeline_root: String) -> CmdResult<PipelineManifest> {
    let root = PathBuf::from(pipeline_root);
    if !root.is_absolute() {
        return Err("pipeline_root must be absolute".into());
    }
    let path = root.join(PIPELINE_MANIFEST_NAME);
    let mut manifest = PipelineManifest::load(&path).stringify_err()?;
    if !pipeline_has_live_owner_evidence(&manifest, &path)
        && manifest.recover_interrupted_supervisor().stringify_err()?
    {
        manifest.persist().stringify_err()?;
    }
    Ok(manifest)
}

#[tauri::command]
pub fn tt_pipeline_list(output_root: String) -> CmdResult<Vec<PipelineListEntry>> {
    let root = PathBuf::from(output_root);
    if !root.is_absolute() {
        return Err("output_root must be absolute".into());
    }
    let mut pipelines = Vec::new();
    for entry in fs::read_dir(&root).stringify_err()? {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let path = entry.path().join(PIPELINE_MANIFEST_NAME);
        let Ok(manifest) = PipelineManifest::load(path) else {
            continue;
        };
        pipelines.push(PipelineListEntry {
            pipeline_id: manifest.pipeline_id,
            output_root: manifest.output_root,
            state: manifest.state,
            updated_at: manifest.updated_at,
            completed_runs: manifest.runs.iter().filter(|run| run.state.terminal()).count(),
            total_runs: manifest.runs.len(),
            candidate_count: manifest
                .runs
                .iter()
                .map(|run| usize::from(run.candidate_ordinal))
                .max()
                .unwrap_or(0),
            repetitions_per_candidate: manifest.repetitions_per_candidate,
        });
    }
    pipelines.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(pipelines)
}

#[tauri::command]
pub async fn tt_pipeline_resume(app_handle: AppHandle, pipeline_root: String) -> CmdResult<PipelineManifest> {
    let root = PathBuf::from(pipeline_root);
    if !root.is_absolute() {
        return Err("pipeline_root must be absolute".into());
    }
    let manifest_path = root.join(PIPELINE_MANIFEST_NAME);
    let mut manifest = PipelineManifest::load(&manifest_path).stringify_err()?;
    if manifest.state != PipelineState::Interrupted {
        return Err("only an interrupted TrafficTracer pipeline can be resumed".into());
    }
    let current_sha = format!("{:x}", Sha256::digest(fs::read(&manifest.config.path).stringify_err()?));
    if current_sha != manifest.config.sha256 {
        return Err("PIPELINE_CONFIG_CHANGED: restore the frozen sites configuration before resume".into());
    }
    let batch = pipeline_batch_from_manifest(&manifest).map_err(smartstring::alias::String::from)?;
    let pipeline_id = manifest.pipeline_id.clone();
    CaptureLock::global()
        .acquire_owned(
            "pipeline",
            pipeline_id.clone(),
            "TrafficTracer profile and proxy pipeline is active",
        )
        .stringify_err()?;
    manifest.restore.state = RestoreState::Pending;
    manifest.restore.error = None;
    manifest.updated_at = Utc::now();
    if let Err(error) = manifest.persist() {
        let _ = CaptureLock::global().release(&pipeline_id);
        return Err(error.to_string().into());
    }
    launch_pipeline_supervisor(app_handle, manifest_path, pipeline_id, batch);
    Ok(manifest)
}

#[tauri::command]
pub async fn tt_pipeline_retry_restore(pipeline_root: String) -> CmdResult<PipelineManifest> {
    let root = PathBuf::from(pipeline_root);
    if !root.is_absolute() {
        return Err("pipeline_root must be absolute".into());
    }
    let manifest_path = root.join(PIPELINE_MANIFEST_NAME);
    let mut manifest = PipelineManifest::load(&manifest_path).stringify_err()?;
    if manifest.state != PipelineState::RestoreFailed {
        return Err("only a restore_failed TrafficTracer pipeline can retry restoration".into());
    }
    if pipeline_runtime().active.lock().is_some() {
        return Err("another TrafficTracer pipeline is active".into());
    }
    let pipeline_id = manifest.pipeline_id.clone();
    CaptureLock::global()
        .acquire_owned(
            "pipeline",
            pipeline_id.clone(),
            "TrafficTracer pipeline restoration is active",
        )
        .stringify_err()?;
    *pipeline_runtime().active.lock() = Some(ActivePipeline {
        pipeline_id: pipeline_id.clone(),
        manifest_path,
        interrupt: Arc::new(AtomicBool::new(false)),
        cancel: Arc::new(AtomicBool::new(false)),
    });
    restore_pipeline(&mut manifest, &pipeline_id).await;
    let _ = CaptureLock::global().release(&pipeline_id);
    let mut active = pipeline_runtime().active.lock();
    if active.as_ref().is_some_and(|item| item.pipeline_id == pipeline_id) {
        *active = None;
    }
    drop(active);
    Ok(manifest)
}

#[tauri::command]
pub async fn tt_pipeline_interrupt(pipeline_id: String) -> CmdResult<PipelineManifest> {
    let (manifest_path, interrupt) = {
        let active = pipeline_runtime().active.lock();
        let item = active
            .as_ref()
            .ok_or_else(|| smartstring::alias::String::from("no active TrafficTracer pipeline"))?;
        if item.pipeline_id != pipeline_id {
            return Err("requested pipeline is not active".into());
        }
        (item.manifest_path.clone(), Arc::clone(&item.interrupt))
    };
    interrupt.store(true, Ordering::Release);
    PipelineManifest::load(manifest_path).stringify_err()
}

#[tauri::command]
pub async fn tt_pipeline_cancel(pipeline_id: String) -> CmdResult<PipelineManifest> {
    let (manifest_path, cancel) = {
        let active = pipeline_runtime().active.lock();
        let item = active
            .as_ref()
            .ok_or_else(|| smartstring::alias::String::from("no active TrafficTracer pipeline"))?;
        if item.pipeline_id != pipeline_id {
            return Err("requested pipeline is not active".into());
        }
        (item.manifest_path.clone(), Arc::clone(&item.cancel))
    };
    cancel.store(true, Ordering::Release);
    PipelineManifest::load(manifest_path).stringify_err()
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
    if batch_status_can_release_capture(&value) {
        let _ = CaptureLock::global().release(&batch_id);
        let _ = manager.mark_ready(&batch_id);
    }
    Ok(value)
}

fn batch_status_can_release_capture(value: &Value) -> bool {
    let batch_terminal = value
        .pointer("/batch/state")
        .and_then(Value::as_str)
        .is_some_and(terminal_batch_state);
    if !batch_terminal {
        return false;
    }

    match value.get("job") {
        None | Some(Value::Null) => true,
        Some(job) => job
            .get("state")
            .and_then(Value::as_str)
            .is_some_and(terminal_batch_state),
    }
}

fn terminal_batch_state(state: &str) -> bool {
    matches!(state, "completed" | "failed" | "cancelled" | "interrupted")
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
pub async fn tt_batch_interrupt(batch_id: String, reason: Option<String>) -> CmdResult<Value> {
    validate_job_id(&batch_id)?;
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(
            RequestMethod::BatchInterrupt,
            BatchStopParams {
                batch_id,
                reason: reason.unwrap_or_else(|| "Interrupted by user.".to_owned()),
            },
        )
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
            BatchStopParams {
                batch_id,
                reason: reason.unwrap_or_else(|| "Cancelled by user.".to_owned()),
            },
        )
        .await
        .stringify_err()
}

#[tauri::command]
pub async fn tt_batch_resume(batch_id: String) -> CmdResult<JobSnapshot> {
    tt_batch_resume_for_owner(batch_id, None).await
}

async fn tt_batch_resume_for_owner(batch_id: String, pipeline_owner: Option<&str>) -> CmdResult<JobSnapshot> {
    validate_job_id(&batch_id)?;
    let manager = WorkerManager::global();
    let lock = CaptureLock::global();
    match pipeline_owner {
        Some(owner) => lock
            .ensure_owned("pipeline", owner, "resuming a pipeline batch")
            .stringify_err()?,
        None => lock
            .acquire(batch_id.clone(), "TrafficTracer batch capture is active")
            .stringify_err()?,
    }
    if let Err(error) = manager.mark_busy(&batch_id) {
        if pipeline_owner.is_none() {
            let _ = lock.release(&batch_id);
        }
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
    finish_batch_request(result, &batch_id, manager, pipeline_owner.is_none()).await
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
    if let Some(playback) = &request.playback {
        if playback.provider != "youtube" {
            return Err("playback provider must be youtube".into());
        }
        if playback.ad_policy != "click_visible_skip" {
            return Err("playback ad_policy must be click_visible_skip".into());
        }
        if playback.desired_primary_seconds == 0 || playback.desired_primary_seconds > request.duration_seconds {
            return Err("playback desired_primary_seconds must fit within duration_seconds".into());
        }
        if !request.options.collect_cdp {
            return Err("playback observation requires CDP collection".into());
        }
        let url = Url::parse(&request.url).map_err(|_| smartstring::alias::String::from("playback URL is invalid"))?;
        let host = url.host_str().unwrap_or_default();
        if host != "youtube.com" && !host.ends_with(".youtube.com") && host != "youtu.be" {
            return Err("youtube playback requires a youtube.com or youtu.be URL".into());
        }
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
    if !valid_pcap_split_mode(&request.options.pcap_split_mode) {
        return Err("pcap_split_mode must be none or unique_connections".into());
    }
    if !valid_cache_mode(&request.options.cache_mode) {
        return Err("cache_mode must be cold or warm".into());
    }
    if !valid_proxy_protocol_mode(&request.options.proxy_protocol_mode) {
        return Err("proxy_protocol_mode must be strict_single or observe".into());
    }
    if !valid_proxy_protocol(&request.options.expected_proxy_protocol) {
        return Err("expected_proxy_protocol must be a protocol name".into());
    }
    if request.options.proxy_selection_group.chars().any(char::is_control) {
        return Err("proxy_selection_group must not contain control characters".into());
    }
    Ok(())
}

fn valid_pcap_split_mode(value: &str) -> bool {
    matches!(value, "none" | "unique_connections")
}

fn valid_cache_mode(value: &str) -> bool {
    matches!(value, "cold" | "warm")
}

fn valid_proxy_protocol_mode(value: &str) -> bool {
    matches!(value, "strict_single" | "observe")
}

fn valid_proxy_protocol(value: &str) -> bool {
    value.is_empty()
        || value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
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
    pub sessions: Vec<SessionSummary>,
    pub corrupt: Vec<CorruptSession>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
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
    pub artifact_count: usize,
    pub warning_count: usize,
    #[serde(default)]
    pub quality_state: Option<String>,
    #[serde(default)]
    pub capture_global_quality_state: Option<String>,
    #[serde(default)]
    pub analysis_integrity_state: Option<String>,
    #[serde(default)]
    pub network_outcome_state: Option<String>,
    #[serde(default)]
    pub scenario_outcome_state: Option<String>,
    #[serde(default)]
    pub navigation_outcome_state: Option<String>,
    #[serde(default)]
    pub navigation_outcome_reason: Option<String>,
    #[serde(default)]
    pub navigation_final_url: Option<String>,
    #[serde(default)]
    pub navigation_final_status: Option<u16>,
    #[serde(default)]
    pub resource_health_state: Option<String>,
    #[serde(default)]
    pub activity_outcome_state: Option<String>,
    #[serde(default)]
    pub coverage: Option<Value>,
    #[serde(default)]
    pub packet_split: Value,
    #[serde(default)]
    pub error: Option<SessionError>,
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
    pub sessions: Vec<SessionSummary>,
    pub corrupt: Vec<CorruptSession>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub has_more: bool,
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
    offset: usize,
    limit: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PacketSplitPreview {
    pub scope: SessionScope,
    pub total: usize,
    pub counts: Value,
    pub missing_only: usize,
    pub repair_incomplete: usize,
    pub sessions: Vec<Value>,
    pub corrupt: Vec<CorruptSession>,
}

#[derive(Serialize)]
struct PacketSplitScopeParams {
    scope_id: String,
}

#[derive(Serialize)]
struct PacketSplitJobParams {
    job: PacketSplitJobSpec,
}

#[derive(Serialize)]
struct PacketSplitJobSpec {
    schema_version: u32,
    kind: &'static str,
    job_id: String,
    scope_id: String,
    output_root: String,
    policy: String,
}

#[derive(Serialize)]
struct SessionListParams {
    offset: usize,
    limit: usize,
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
pub async fn tt_session_list(offset: usize, limit: usize) -> CmdResult<SessionListResult> {
    validate_session_page(limit)?;
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(RequestMethod::SessionList, SessionListParams { offset, limit })
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
pub async fn tt_session_scope_list(
    scope_id: String,
    offset: usize,
    limit: usize,
) -> CmdResult<ScopedSessionListResult> {
    if scope_id.trim().is_empty() {
        return Err("scope_id must not be empty".into());
    }
    validate_session_page(limit)?;
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(
            RequestMethod::SessionScopeList,
            SessionScopeIdParams {
                scope_id,
                offset,
                limit,
            },
        )
        .await
        .stringify_err()
}

#[tauri::command]
pub async fn tt_packet_split_preview(scope_id: String) -> CmdResult<PacketSplitPreview> {
    if scope_id.trim().is_empty() {
        return Err("scope_id must not be empty".into());
    }
    WorkerManager::global()
        .client()
        .stringify_err()?
        .request(
            RequestMethod::SessionScopePacketSplitPreview,
            PacketSplitScopeParams { scope_id },
        )
        .await
        .stringify_err()
}

#[tauri::command]
pub async fn tt_packet_split_start(scope_id: String, policy: String) -> CmdResult<JobSnapshot> {
    if scope_id.trim().is_empty() {
        return Err("scope_id must not be empty".into());
    }
    if !matches!(policy.as_str(), "missing_only" | "repair_incomplete") {
        return Err("unsupported packet split policy".into());
    }
    let manager = WorkerManager::global();
    let root = manager.session_root().stringify_err()?;
    let job_id = new_job_id()?;
    let client = manager.client().stringify_err()?;
    manager.mark_busy(&job_id).stringify_err()?;
    let result = client
        .request::<_, JobSnapshot>(
            RequestMethod::PacketSplitStart,
            PacketSplitJobParams {
                job: PacketSplitJobSpec {
                    schema_version: JOB_SCHEMA_VERSION,
                    kind: "packet_split_group",
                    job_id: job_id.clone(),
                    scope_id,
                    output_root: root.to_string_lossy().into_owned(),
                    policy,
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

fn validate_session_page(limit: usize) -> CmdResult {
    if !(1..=100).contains(&limit) {
        return Err("limit must be between 1 and 100".into());
    }
    Ok(())
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
        .unwrap_or_else(|| {
            session_dir
                .join(if session_dir.join("raw").is_dir() {
                    "analysis"
                } else {
                    "results"
                })
                .join(filename)
        });

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
            "corrupt": [],
            "offset": 0,
            "limit": 20,
            "total": 0,
            "has_more": false
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
            "corrupt": [],
            "offset": 0,
            "limit": 20,
            "total": 0,
            "has_more": false
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

    #[test]
    fn semantic_runtime_fingerprint_ignores_mapping_order_and_formatting() {
        let left =
            semantic_runtime_fingerprint(b"mixed-port: 7890\ndns:\n  enable: true\n  nameserver: [1.1.1.1, 8.8.8.8]\n")
                .unwrap();
        let right =
            semantic_runtime_fingerprint(b"dns: {nameserver: [1.1.1.1, 8.8.8.8], enable: true}\nmixed-port: 7890\n")
                .unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn semantic_runtime_fingerprint_detects_behavioral_change() {
        let left = semantic_runtime_fingerprint(b"tun: {enable: true, device: Meta}\n").unwrap();
        let right = semantic_runtime_fingerprint(b"tun: {enable: false, device: Meta}\n").unwrap();
        assert_ne!(left, right);
    }

    fn proxy_snapshot(node: &str, leaf: &str, protocol: &str) -> PipelineProxySnapshot {
        PipelineProxySnapshot {
            profile_uid: "profile-one".into(),
            profile_fingerprint: "a".repeat(64),
            selection_group: "GLOBAL".into(),
            selected_node: node.into(),
            resolved_chain: if node == leaf {
                vec![node.into()]
            } else {
                vec![node.into(), leaf.into()]
            },
            resolved_leaf: leaf.into(),
            protocol: protocol.into(),
            captured_at: Utc::now(),
        }
    }

    #[test]
    fn resolves_selector_chain_into_a_frozen_proxy_snapshot() {
        let proxies: Proxies = serde_json::from_value(serde_json::json!({
            "proxies": {
                "GLOBAL": {"name":"GLOBAL", "type":"Selector", "now":"edge", "all":["edge"]},
                "edge": {"name":"edge", "type":"Selector", "now":"leaf"},
                "leaf": {"name":"leaf", "type":"Hysteria2"}
            }
        }))
        .unwrap();
        let snapshot = proxy_snapshot_from_runtime(&proxies, "profile-one".into(), "a".repeat(64), "GLOBAL").unwrap();
        assert_eq!(snapshot.selected_node, "edge");
        assert_eq!(snapshot.resolved_chain, ["edge", "leaf"]);
        assert_eq!(snapshot.resolved_leaf, "leaf");
        assert_eq!(snapshot.protocol, "hysteria2");
    }

    #[test]
    fn classifies_node_protocol_drift_and_missing_observation_separately() {
        let start = proxy_snapshot("edge", "leaf-one", "hysteria2");
        let mut matched = ObservedRunEvidence::default();
        matched.contexts = 2;
        matched.protocols.insert("hysteria2".into());
        matched.selected_nodes.insert("edge".into());
        matched.leaf_nodes.insert("leaf-one".into());
        let passed = classify_run_verification(&start, Some(&start), &matched, None);
        assert_eq!(passed.node_state, "passed");
        assert_eq!(passed.protocol_state, "passed");

        let end = proxy_snapshot("other", "leaf-two", "vless");
        let mut drifted = ObservedRunEvidence::default();
        drifted.contexts = 1;
        drifted.protocols.insert("vless".into());
        drifted.selected_nodes.insert("other".into());
        drifted.leaf_nodes.insert("leaf-two".into());
        let drift = classify_run_verification(&start, Some(&end), &drifted, None);
        assert_eq!(drift.node_state, "node_drift");
        assert_eq!(drift.protocol_state, "protocol_mismatch");

        let unavailable = classify_run_verification(
            &start,
            None,
            &ObservedRunEvidence::default(),
            Some(&PipelineBarrierError::new("CONTROLLER_UNAVAILABLE", "offline")),
        );
        assert_eq!(unavailable.node_state, "observation_unavailable");
        assert_eq!(unavailable.protocol_state, "observation_unavailable");
    }

    #[test]
    fn preserves_structured_pipeline_barrier_error_codes() {
        let error = pipeline_run_error("CONNECTION_DRAIN_FAILED: old connections remain".into());
        assert_eq!(error.code, "CONNECTION_DRAIN_FAILED");
        let fallback = pipeline_run_error("unstructured controller error".into());
        assert_eq!(fallback.code, "PIPELINE_RUN_FAILED");
    }

    #[test]
    fn classifies_shared_contract_failures_as_systemic() {
        let contract = pipeline_run_error("Worker returned InvalidParams: CONTRACT_VALIDATION_FAILED: job/kind".into());
        assert!(systemic_pipeline_error(&contract));
        assert!(systemic_pipeline_error(&PipelineError {
            code: "PIPELINE_BATCH_PREFLIGHT_FAILED".into(),
            message: "Worker method mismatch".into(),
        }));
        assert!(!systemic_pipeline_error(&pipeline_run_error(
            "MAIN_DOCUMENT_NETWORK_ERROR: target failed".into(),
        )));
    }

    #[test]
    fn pipeline_quality_separates_application_failure_from_valid_correlation() {
        let root = std::env::temp_dir().join(format!("traffictracer-pipeline-quality-{}", std::process::id()));
        let write_session = |name: &str, summary: Value, target_url: &str| {
            let session = root.join(name);
            fs::create_dir_all(session.join("analysis")).unwrap();
            fs::create_dir_all(session.join("raw")).unwrap();
            fs::write(
                session.join("analysis/summary.json"),
                serde_json::to_vec(&summary).unwrap(),
            )
            .unwrap();
            fs::write(
                session.join("raw/capture-context.json"),
                serde_json::to_vec(&serde_json::json!({
                    "target": {"url": target_url}
                }))
                .unwrap(),
            )
            .unwrap();
        };
        write_session(
            "youtube-good",
            serde_json::json!({
                "session_id": "session-good",
                "quality_state": "passed",
                "analysis_integrity": {"page_attributed": {"state": "passed"}},
                "scenario_outcome": {
                    "state": "passed",
                    "primary_content_seconds": 28.083,
                    "desired_primary_seconds": 25
                }
            }),
            "https://www.youtube.com/watch?v=good",
        );
        write_session(
            "youtube-failed",
            serde_json::json!({
                "session_id": "session-failed",
                "quality_state": "passed",
                "analysis_integrity": {"page_attributed": {"state": "passed"}},
                "scenario_outcome": {
                    "state": "failed",
                    "reason": "PLAYER_NOT_CREATED",
                    "primary_content_seconds": 0.0,
                    "desired_primary_seconds": 25
                },
                "playback": {"diagnostics": {"last_observation": {
                    "href": "https://www.google.com/sorry/"
                }}}
            }),
            "https://www.youtube.com/watch?v=failed",
        );
        write_session(
            "example",
            serde_json::json!({
                "session_id": "session-example",
                "quality_state": "passed",
                "analysis_integrity": {"page_attributed": {"state": "passed"}},
                "activity_outcome": {
                    "state": "not_applicable"
                }
            }),
            "https://example.com/",
        );

        let quality = pipeline_run_quality(&root, None);
        assert_eq!(quality.sessions_total, 3);
        assert_eq!(quality.capture_integrity.state, "passed");
        assert_eq!(quality.correlation.state, "passed");
        assert_eq!(quality.application.state, "failed");
        assert_eq!(quality.application.passed, 1);
        assert_eq!(quality.application.failed, 1);
        assert_eq!(quality.application.not_applicable, 1);
        assert!(run_quality_requires_attention(&quality));
        assert_eq!(quality.application_issues.len(), 1);
        assert_eq!(
            quality.application_issues[0].reason.as_deref(),
            Some("PLAYER_NOT_CREATED")
        );
        assert_eq!(
            quality.application_issues[0].final_url.as_deref(),
            Some("https://www.google.com/sorry/")
        );
        assert_eq!(quality.application_issues[0].primary_content_millis, Some(0));
        let effective = HashSet::from(["session-good".to_owned(), "session-example".to_owned()]);
        let retried = pipeline_run_quality(&root, Some(&effective));
        assert_eq!(retried.sessions_total, 2);
        assert_eq!(retried.application.state, "passed");
        assert!(retried.application_issues.is_empty());
        assert!(!run_quality_requires_attention(&retried));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pipeline_quality_surfaces_generic_navigation_failure() {
        let root = std::env::temp_dir().join(format!("traffictracer-navigation-quality-{}", std::process::id()));
        let session = root.join("github-404");
        fs::create_dir_all(session.join("analysis")).unwrap();
        fs::create_dir_all(session.join("raw")).unwrap();
        fs::write(
            session.join("analysis/summary.json"),
            serde_json::to_vec(&serde_json::json!({
                "session_id": "session-github",
                "quality_state": "passed",
                "analysis_integrity": {"page_attributed": {"state": "passed"}},
                "activity_outcome": {
                    "kind": "page_load",
                    "state": "failed",
                    "reason": "MAIN_DOCUMENT_HTTP_ERROR",
                    "final_url": "https://github.com/private/repository",
                    "final_status": 404
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            session.join("raw/capture-context.json"),
            br#"{"target":{"url":"https://github.com/private/repository"}}"#,
        )
        .unwrap();

        let quality = pipeline_run_quality(&root, None);

        assert_eq!(quality.capture_integrity.state, "passed");
        assert_eq!(quality.application.state, "failed");
        assert_eq!(quality.application_issues.len(), 1);
        assert_eq!(quality.application_issues[0].final_status, Some(404));
        assert_eq!(
            quality.application_issues[0].reason.as_deref(),
            Some("MAIN_DOCUMENT_HTTP_ERROR")
        );
        let _ = fs::remove_dir_all(root);
    }

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
            playback: None,
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
    fn capture_spec_rejects_invalid_pcap_split_mode() {
        let mut request = valid_request();
        request.options.pcap_split_mode = "compressed".to_owned();
        assert!(validate_capture_request(&request).is_err());
    }

    #[test]
    fn capture_spec_rejects_invalid_cache_mode() {
        let mut request = valid_request();
        request.options.cache_mode = "stale".to_owned();
        assert!(validate_capture_request(&request).is_err());
    }

    #[test]
    fn capture_spec_rejects_invalid_proxy_protocol_options() {
        let mut request = valid_request();
        request.options.proxy_protocol_mode = "guess".to_owned();
        assert!(validate_capture_request(&request).is_err());

        request.options.proxy_protocol_mode = "strict_single".to_owned();
        request.options.expected_proxy_protocol = "hy2 invalid".to_owned();
        assert!(validate_capture_request(&request).is_err());
    }

    #[test]
    fn capture_spec_validates_bounded_youtube_playback() {
        let mut request = valid_request();
        request.url = "https://www.youtube.com/watch?v=test".to_owned();
        request.domain = "youtube.com".to_owned();
        request.duration_seconds = 35;
        request.playback = Some(PlaybackPolicy {
            provider: "youtube".to_owned(),
            ad_policy: "click_visible_skip".to_owned(),
            desired_primary_seconds: 25,
        });
        validate_capture_request(&request).unwrap();

        request.playback.as_mut().unwrap().desired_primary_seconds = 36;
        assert!(validate_capture_request(&request).is_err());
        request.playback.as_mut().unwrap().desired_primary_seconds = 25;

        request.options.collect_cdp = false;
        assert!(validate_capture_request(&request).is_err());
        request.options.collect_cdp = true;

        request.url = "https://example.com/watch?v=test".to_owned();
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

    #[test]
    fn ambiguous_batch_start_errors_retain_capture_ownership_for_reconciliation() {
        use crate::core::traffic_tracer::{
            client::ClientError,
            protocol::{RequestId, WorkerErrorCode},
        };

        for error in [
            ClientError::Transport("injected response loss".to_owned()),
            ClientError::Timeout(RequestId::Integer(7)),
            ClientError::Decode("injected decode failure".to_owned()),
            ClientError::Protocol("injected protocol failure".to_owned()),
            ClientError::WorkerExited,
        ] {
            assert!(batch_request_outcome_unknown(&error), "{error}");
        }
        assert!(!batch_request_outcome_unknown(&ClientError::Worker {
            code: WorkerErrorCode::InvalidParams,
            message: "rejected before acceptance".to_owned(),
            data: None,
        }));
        assert!(!batch_request_outcome_unknown(&ClientError::Encode(
            "request was not sent".to_owned()
        )));

        let snapshot = uncertain_batch_start_snapshot("00000000-0000-4000-8000-000000000001");
        assert_eq!(snapshot.state, JobState::Created);
        assert_eq!(snapshot.stage, "starting_batch");
        assert!(!snapshot.state.terminal());
    }

    #[test]
    fn pipeline_owner_heartbeat_requires_matching_fresh_live_supervisor() {
        let now = unix_time_ms();
        let mut record = PipelineOwnerRecord {
            schema_version: 1,
            pipeline_id: "pipeline-one".to_owned(),
            app_pid: std::process::id(),
            state: "supervising".to_owned(),
            stage: "running_batch".to_owned(),
            batch_id: Some("batch-one".to_owned()),
            heartbeat_at_ms: now,
        };
        assert!(owner_record_is_live(&record, "pipeline-one", now));

        record.state = "released".to_owned();
        assert!(!owner_record_is_live(&record, "pipeline-one", now));
        record.state = "supervising".to_owned();
        assert!(!owner_record_is_live(&record, "pipeline-two", now));
        assert!(!owner_record_is_live(
            &record,
            "pipeline-one",
            now.saturating_add(PIPELINE_OWNER_HEARTBEAT_FRESH_MS + 1)
        ));
    }

    #[test]
    fn pipeline_reuses_batch_validation_before_creating_a_supervisor() {
        let mut request = batch_request(vec![batch_target(0, "example.com")]);
        assert!(validate_batch_start_request(&request).is_ok());

        request.targets.clear();
        assert!(validate_batch_start_request(&request).is_err());
        request.targets = vec![batch_target(0, "example.com")];
        request.options.analyze_after_capture = false;
        assert!(validate_batch_start_request(&request).is_err());
    }

    #[test]
    fn stop_requests_have_identical_semantics_at_every_pipeline_stage() {
        let interrupt = AtomicBool::new(false);
        let cancel = AtomicBool::new(false);
        let stages = [
            PipelineStage::ActivatingProfile,
            PipelineStage::SelectingProxy,
            PipelineStage::DrainingConnections,
            PipelineStage::Preflight,
            PipelineStage::StartingBatch,
            PipelineStage::RunningBatch,
            PipelineStage::FinalizingBatch,
            PipelineStage::Restoring,
        ];
        for _stage in stages {
            interrupt.store(false, Ordering::Release);
            cancel.store(false, Ordering::Release);
            assert_eq!(requested_pipeline_stop(&interrupt, &cancel), None);
            interrupt.store(true, Ordering::Release);
            assert_eq!(
                requested_pipeline_stop(&interrupt, &cancel),
                Some(PipelineRunState::Interrupted)
            );
            cancel.store(true, Ordering::Release);
            assert_eq!(
                requested_pipeline_stop(&interrupt, &cancel),
                Some(PipelineRunState::Cancelled),
                "cancel must win when both requests arrive"
            );
        }
    }

    #[test]
    fn resumed_batch_does_not_release_capture_for_an_active_job() {
        for state in ["created", "preparing", "capturing", "analyzing"] {
            let value = serde_json::json!({
                "batch": {"state": "failed"},
                "job": {"state": state},
            });
            assert!(!batch_status_can_release_capture(&value), "job state {state}");
        }
    }

    #[test]
    fn terminal_batch_releases_capture_only_when_job_is_terminal_or_absent() {
        for state in ["completed", "failed", "cancelled", "interrupted"] {
            let terminal_job = serde_json::json!({
                "batch": {"state": "completed"},
                "job": {"state": state},
            });
            assert!(batch_status_can_release_capture(&terminal_job));
        }
        assert!(batch_status_can_release_capture(&serde_json::json!({
            "batch": {"state": "failed"},
            "job": null,
        })));
        assert!(!batch_status_can_release_capture(&serde_json::json!({
            "batch": {"state": "running"},
            "job": {"state": "completed"},
        })));
        assert!(!batch_status_can_release_capture(&serde_json::json!({
            "batch": {"state": "failed"},
            "job": {},
        })));
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
            playback: None,
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
            application_retry: ApplicationRetryPolicy::default(),
        }
    }

    #[test]
    fn batch_job_serialization_omits_absent_playback() {
        let plain = batch_target(0, "example.com");
        let mut youtube = batch_target(1, "youtube.com");
        youtube.playback = Some(PlaybackPolicy {
            provider: "youtube".to_owned(),
            ad_policy: "click_visible_skip".to_owned(),
            desired_primary_seconds: 25,
        });

        let value = serde_json::to_value(BatchJobSpec {
            schema_version: JOB_SCHEMA_VERSION,
            kind: "batch",
            job_id: "123e4567-e89b-42d3-a456-426614174000".to_owned(),
            config_path: "/tmp/targets.yaml".to_owned(),
            config_sha256: "a".repeat(64),
            targets: vec![plain, youtube],
            interfaces: CaptureInterfaces {
                tun: "Meta".to_owned(),
                physical: "eth0".to_owned(),
            },
            output_root: "/tmp/sessions".to_owned(),
            chrome_binary: "/usr/bin/chromium".to_owned(),
            controller: CaptureController {
                endpoint: "unix:///tmp/mihomo.sock".to_owned(),
                secret: String::new(),
            },
            options: CaptureOptions::default(),
            fail_fast: true,
            application_retry: ApplicationRetryPolicy::default(),
            orchestration: None,
        })
        .unwrap();

        assert_eq!(value["kind"], "batch");
        assert_eq!(
            value["application_retry"],
            serde_json::json!({"enabled": false, "max_retries": 1})
        );
        assert!(value["targets"][0].get("playback").is_none());
        assert_eq!(
            value["targets"][1]["playback"],
            serde_json::json!({
                "provider": "youtube",
                "ad_policy": "click_visible_skip",
                "desired_primary_seconds": 25,
            })
        );
    }

    #[test]
    fn batch_retry_policy_is_bounded_to_one() {
        let mut request = batch_request(vec![batch_target(0, "youtube.com")]);
        request.application_retry = ApplicationRetryPolicy {
            enabled: true,
            max_retries: 2,
        };
        assert!(validate_batch_start_request(&request).is_err());
        request.application_retry.max_retries = 1;
        assert!(validate_batch_start_request(&request).is_ok());
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
