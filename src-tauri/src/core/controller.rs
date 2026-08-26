use crate::{
    config::IClashTemp,
    core::{handle::Handle, owner_identity::current_owner_identity},
};
use anyhow::{Context as _, Result, bail};
use arc_swap::ArcSwap;
use clash_verge_logging::{Type, logging};
use once_cell::sync::Lazy;
use std::{sync::Arc, time::Duration};
use tauri_plugin_mihomo::MihomoExt as _;

const READY_ATTEMPTS: usize = 20;
const READY_RETRY_DELAY: Duration = Duration::from_millis(250);
const READY_REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

static ACTIVE_CONTROLLER_IPC: Lazy<ArcSwap<String>> =
    Lazy::new(|| ArcSwap::from_pointee(IClashTemp::guard_external_controller_ipc().to_string()));

/// Return the IPC endpoint used by every in-process Mihomo API consumer.
pub fn active_ipc_path() -> String {
    ACTIVE_CONTROLLER_IPC.load_full().as_ref().clone()
}

pub fn sidecar_ipc_path() -> String {
    IClashTemp::guard_external_controller_ipc().to_string()
}

pub fn service_ipc_path() -> Result<String> {
    Ok(clash_verge_service_ipc::mihomo_ipc_path(&current_owner_identity()?))
}

/// Switch all API clients during a core-mode transition.
///
/// WebSockets and the pooled local-socket clients must both be discarded;
/// otherwise requests created after a service/sidecar switch can continue to
/// use the previous core's socket.
pub async fn activate_ipc_path(path: String) -> Result<()> {
    let app = Handle::app_handle();
    let mut mihomo = app.mihomo().write().await;
    mihomo
        .clear_all_ws_connections()
        .await
        .context("failed to clear Mihomo WebSocket connections")?;
    mihomo
        .update_socket_path(path.clone())
        .context("failed to update Mihomo controller IPC path")?;
    ACTIVE_CONTROLLER_IPC.store(Arc::new(path.clone()));
    logging!(info, Type::Core, "Mihomo controller IPC switched to {path}");
    Ok(())
}

/// Wait until the selected controller answers a real Mihomo API request.
/// Socket creation alone is insufficient: it does not prove the UI and core
/// agree on the endpoint or that the core finished booting.
pub async fn activate_and_wait(path: String) -> Result<()> {
    activate_ipc_path(path.clone()).await?;

    let mut last_error = String::from("controller did not answer");
    for attempt in 1..=READY_ATTEMPTS {
        let probe = async { Handle::mihomo().await.get_version().await };
        match tokio::time::timeout(READY_REQUEST_TIMEOUT, probe).await {
            Ok(Ok(version)) => {
                logging!(
                    info,
                    Type::Core,
                    "Mihomo controller ready at {path} (version: {:?}, attempt: {attempt})",
                    version
                );
                // Queries may have cached transient errors while the controller
                // was switching (for example, sidecar -> service). Notify the
                // frontend only after the new controller passes a real API probe.
                Handle::refresh_clash();
                return Ok(());
            }
            Ok(Err(error)) => last_error = error.to_string(),
            Err(_) => last_error = format!("probe timed out after {READY_REQUEST_TIMEOUT:?}"),
        }
        tokio::time::sleep(READY_RETRY_DELAY).await;
    }

    bail!("Mihomo controller at {path} was not ready after {READY_ATTEMPTS} attempts: {last_error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_verge_service_ipc::OwnerIdentity;

    #[test]
    fn mihomo_plugin_accepts_tuic_config_without_ech_key() {
        let raw = r#"{
            "enable": false,
            "listen": "",
            "certificate": "",
            "private-key": ""
        }"#;
        let config = serde_json::from_str::<tauri_plugin_mihomo::models::TuicServer>(raw);
        assert!(config.is_ok(), "TUIC compatibility parse failed: {:?}", config.err());
    }

    #[test]
    #[cfg(unix)]
    fn service_endpoint_is_scoped_to_the_owner_uid() {
        let endpoint = clash_verge_service_ipc::mihomo_ipc_path(&OwnerIdentity::Unix { uid: 4242, gid: 4242 });
        assert!(endpoint.ends_with("/users/4242/verge-mihomo.sock"));
    }

    #[test]
    fn active_endpoint_defaults_to_sidecar_endpoint() {
        assert_eq!(active_ipc_path(), sidecar_ipc_path());
    }
}
