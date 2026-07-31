use std::{sync::Arc, time::Duration};

use anyhow::{Result, bail};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::AppHandle;

use super::{
    client::WorkerClient,
    worker::{WorkerEvent, WorkerProcess},
};
use crate::singleton;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WorkerManagerState {
    Stopped,
    Starting,
    Ready,
    Busy,
    Failed { message: String },
}

pub struct WorkerManager {
    state: Arc<Mutex<WorkerManagerState>>,
    process: Arc<WorkerProcess>,
    client: Mutex<Option<Arc<WorkerClient>>>,
    bridge: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    monitor: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
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

    pub async fn start(&self, app_handle: &AppHandle) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        self.begin_start()?;

        let client = Arc::new(WorkerClient::new(Arc::clone(&self.process), DEFAULT_REQUEST_TIMEOUT));
        let bridge = self.process.bridge_to_tauri(app_handle.clone());
        let monitor = self.spawn_exit_monitor();

        if let Err(error) = self.process.start(app_handle) {
            bridge.abort();
            monitor.abort();
            self.fail_start(error.to_string());
            return Err(error);
        }

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
        self.finish_start()
    }

    pub async fn stop(&self) -> Result<bool> {
        let _lifecycle = self.lifecycle.lock().await;
        self.client.lock().take();
        if let Some(bridge) = self.bridge.lock().take() {
            bridge.abort();
        }
        if let Some(monitor) = self.monitor.lock().take() {
            monitor.abort();
        }
        let stopped = self.process.stop()?;
        *self.state.lock() = WorkerManagerState::Stopped;
        Ok(stopped)
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
        *self.state.lock() = WorkerManagerState::Failed { message };
    }

    fn spawn_exit_monitor(&self) -> tauri::async_runtime::JoinHandle<()> {
        let mut events = self.process.subscribe();
        let state = Arc::clone(&self.state);
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
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

singleton!(WorkerManager, TRAFFIC_TRACER_WORKER_MANAGER);

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

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
