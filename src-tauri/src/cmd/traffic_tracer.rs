use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::AppHandle;

use super::{CmdResult, StringifyErr as _};
use crate::{
    config::{Config, IClashTemp},
    core::{
        service,
        traffic_tracer::{
            manager::{WorkerManager, WorkerManagerState},
            protocol::RequestMethod,
        },
    },
};

const TRAFFIC_TRACER_CORE: &str = "verge-mihomo-tt";
const DEFAULT_CHROME_BINARY: &str = "google-chrome";

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
    let controller_secret = clash.get_client_info().secret.unwrap_or_default();
    drop(clash);

    let service_available = service::is_service_available().await.is_ok();
    let manager = WorkerManager::global();
    if matches!(
        manager.state(),
        WorkerManagerState::Stopped | WorkerManagerState::Failed { .. }
    ) {
        manager.start(&app_handle).await.stringify_err()?;
    }
    let client = manager.client().stringify_err()?;
    let worker_report = client
        .request::<_, WorkerDiagnosticReport>(
            RequestMethod::EnvironmentDiagnose,
            WorkerEnvironmentParams {
                controller_endpoint: local_controller_endpoint(),
                controller_secret,
                tun_interface: request.tun_interface,
                physical_interface: request.physical_interface,
                chrome_binary: if request.chrome_binary.trim().is_empty() {
                    DEFAULT_CHROME_BINARY.to_owned()
                } else {
                    request.chrome_binary
                },
                output_root: request.output_root,
                min_free_bytes: request.min_free_bytes,
            },
        )
        .await
        .stringify_err()?;

    Ok(merge_environment(
        worker_report,
        CompleteIntegrationStatus {
            current_core,
            tun_enabled,
            service_available,
            worker: manager.state(),
        },
    ))
}

fn local_controller_endpoint() -> String {
    let endpoint = IClashTemp::guard_external_controller_ipc();
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
    #[serde(default)]
    pub options: CaptureOptions,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureNetwork {
    Tcp,
    Udp,
    All,
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
    manager.mark_busy().stringify_err()?;
    let client = manager.client().stringify_err()?;
    let controller_secret = Config::clash()
        .await
        .latest_arc()
        .get_client_info()
        .secret
        .unwrap_or_default();
    let job_id = new_job_id()?;
    let result = client
        .request::<_, JobSnapshot>(
            RequestMethod::JobStart,
            CaptureJobParams {
                job: CaptureJobSpec {
                    schema_version: 1,
                    kind: "capture",
                    job_id,
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
                },
            },
        )
        .await;

    match result {
        Ok(snapshot) => Ok(snapshot),
        Err(error) => {
            let _ = manager.mark_ready();
            Err(error.to_string().into())
        }
    }
}

#[tauri::command]
pub async fn tt_capture_get(job_id: String) -> CmdResult<JobSnapshot> {
    validate_job_id(&job_id)?;
    let manager = WorkerManager::global();
    let snapshot = manager
        .client()
        .stringify_err()?
        .request::<_, JobSnapshot>(RequestMethod::JobStatus, JobIdParams { job_id })
        .await
        .stringify_err()?;
    if snapshot.state.terminal() {
        let _ = manager.mark_ready();
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
                job_id,
                reason: reason.unwrap_or_else(|| "Cancelled by user.".to_owned()),
            },
        )
        .await
        .stringify_err()?;
    if snapshot.state.terminal() {
        let _ = manager.mark_ready();
    }
    Ok(snapshot)
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
}
#[cfg(test)]
mod tests {
    use super::*;

    fn integration(core: &str, tun_enabled: bool, service_available: bool) -> CompleteIntegrationStatus {
        CompleteIntegrationStatus {
            current_core: core.to_owned(),
            tun_enabled,
            service_available,
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
