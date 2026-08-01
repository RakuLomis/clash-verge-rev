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
