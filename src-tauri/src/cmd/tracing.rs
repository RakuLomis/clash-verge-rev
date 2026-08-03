use super::CmdResult;
use super::StringifyErr as _;
use crate::core::controller;
use crate::core::traffic_tracer::lock::CaptureLock;
use reqwest::{Client, ClientBuilder};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const TRACING_URL: &str = "http://localhost/experimental/tracing";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracingState {
    pub enabled: bool,
    #[serde(default)]
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracingPatch {
    pub enabled: Option<bool>,
    pub output: Option<String>,
}

fn tracing_client() -> Result<Client, String> {
    let socket_path = controller::active_ipc_path();
    let builder = ClientBuilder::new().timeout(REQUEST_TIMEOUT);
    #[cfg(unix)]
    let builder = builder.unix_socket(socket_path);
    #[cfg(windows)]
    let builder = builder.windows_named_pipe(socket_path);
    builder.build().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_tracing_state() -> CmdResult<TracingState> {
    let response = tracing_client()?
        .get(TRACING_URL)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("mihomo returned {}", response.status()).into());
    }
    response.json().await.map_err(|error| error.to_string().into())
}

#[tauri::command]
pub async fn patch_tracing_state(payload: TracingPatch) -> CmdResult<TracingState> {
    CaptureLock::global()
        .ensure_unlocked("changing manual tracing settings")
        .stringify_err()?;
    let response = tracing_client()?
        .patch(TRACING_URL)
        .json(&payload)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("mihomo returned {}", response.status()).into());
    }
    response.json().await.map_err(|error| error.to_string().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracing_uses_local_controller_url() {
        assert_eq!(TRACING_URL, "http://localhost/experimental/tracing");
    }
}
