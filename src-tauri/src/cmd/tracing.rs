use super::CmdResult;
use crate::config::Config;
use serde::{Deserialize, Serialize};

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

async fn tracing_url() -> (String, Option<String>) {
    let clash_info = Config::clash().await.data_arc().get_client_info();
    let url = format!("http://{}/experimental/tracing", clash_info.server);
    (url, clash_info.secret)
}

#[tauri::command]
pub async fn get_tracing_state() -> CmdResult<TracingState> {
    let (url, secret) = tracing_url().await;
    let client = reqwest::Client::new();
    let mut req = client.get(&url);
    if let Some(ref s) = secret {
        req = req.header("Authorization", format!("Bearer {}", s));
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("mihomo returned {}", resp.status()).into());
    }
    let state: TracingState = resp.json().await.map_err(|e| e.to_string())?;
    Ok(state)
}

#[tauri::command]
pub async fn patch_tracing_state(payload: TracingPatch) -> CmdResult<TracingState> {
    let (url, secret) = tracing_url().await;
    let client = reqwest::Client::new();
    let mut req = client.patch(&url);
    if let Some(ref s) = secret {
        req = req.header("Authorization", format!("Bearer {}", s));
    }
    let body = serde_json::to_value(&payload).map_err(|e| e.to_string())?;
    let resp = req.json(&body).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("mihomo returned {}", resp.status()).into());
    }
    let state: TracingState = resp.json().await.map_err(|e| e.to_string())?;
    Ok(state)
}
