use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context as _, Result, bail};
use clash_verge_logging::Type;
use parking_lot::Mutex;
use tauri::AppHandle;
use tauri_plugin_shell::{
    ShellExt as _,
    process::{CommandChild, CommandEvent, TerminatedPayload},
};
use tokio::sync::{broadcast, mpsc};

use super::events::TauriEventBridge;
use crate::logging;

const WORKER_SIDECAR_NAME: &str = "traffictracer-worker";
const EVENT_BUFFER_SIZE: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerEvent {
    Stdout { instance_id: u64, line: String },
    MalformedStdout { instance_id: u64, error: String },
    Stderr { instance_id: u64, line: String },
    TransportError { instance_id: u64, error: String },
    Exited { instance_id: u64, status: WorkerExit },
}

pub(super) trait ManagedChild: Send {
    fn pid(&self) -> u32;
    fn write(&mut self, bytes: &[u8]) -> Result<()>;
    fn kill(self: Box<Self>) -> Result<()>;
}

impl ManagedChild for CommandChild {
    fn pid(&self) -> u32 {
        CommandChild::pid(self)
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        CommandChild::write(self, bytes).context("failed to write to TrafficTracer Worker")
    }

    fn kill(self: Box<Self>) -> Result<()> {
        (*self).kill().context("failed to stop TrafficTracer Worker")
    }
}

struct RunningChild {
    instance_id: u64,
    child: Box<dyn ManagedChild>,
}

#[derive(Default)]
struct ProcessState {
    child: Option<RunningChild>,
}

pub struct WorkerProcess {
    state: Arc<Mutex<ProcessState>>,
    next_instance_id: AtomicU64,
    events: broadcast::Sender<WorkerEvent>,
}

impl Default for WorkerProcess {
    fn default() -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER_SIZE);
        Self {
            state: Arc::new(Mutex::new(ProcessState::default())),
            next_instance_id: AtomicU64::new(1),
            events,
        }
    }
}

impl WorkerProcess {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self, app_handle: &AppHandle, output_root: &Path) -> Result<u64> {
        let mut state = self.state.lock();
        if state.child.is_some() {
            bail!("TrafficTracer Worker is already running");
        }

        let (receiver, child) = app_handle
            .shell()
            .sidecar(WORKER_SIDECAR_NAME)
            .context("failed to resolve TrafficTracer Worker sidecar")?
            .args(["--output-root", &output_root.to_string_lossy()])
            .spawn()
            .context("failed to start TrafficTracer Worker sidecar")?;
        let instance_id = self.next_instance_id.fetch_add(1, Ordering::Relaxed);
        let pid = child.pid();
        state.child = Some(RunningChild {
            instance_id,
            child: Box::new(child),
        });
        drop(state);

        logging!(
            info,
            Type::System,
            "Started TrafficTracer Worker instance {} (PID {})",
            instance_id,
            pid
        );
        Self::watch_events(Arc::clone(&self.state), self.events.clone(), instance_id, receiver);
        Ok(instance_id)
    }

    pub fn stop(&self) -> Result<bool> {
        let running = self.state.lock().child.take();
        let Some(running) = running else {
            return Ok(false);
        };

        logging!(
            info,
            Type::System,
            "Stopping TrafficTracer Worker instance {} (PID {})",
            running.instance_id,
            running.child.pid()
        );
        running.child.kill()?;
        Ok(true)
    }

    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        let mut state = self.state.lock();
        let running = state.child.as_mut().context("TrafficTracer Worker is not running")?;
        running.child.write(bytes)
    }

    pub fn is_running(&self) -> bool {
        self.state.lock().child.is_some()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WorkerEvent> {
        self.events.subscribe()
    }

    pub fn bridge_to_tauri(&self, app_handle: AppHandle) -> tauri::async_runtime::JoinHandle<()> {
        let mut receiver = self.subscribe();
        let bridge = TauriEventBridge::new(app_handle);
        tauri::async_runtime::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(WorkerEvent::Stdout { line, .. }) => bridge.handle_line(&line),
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        logging!(
                            warn,
                            Type::Frontend,
                            "TrafficTracer event bridge missed {} process events",
                            count
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }

    pub(super) fn attach(&self, receiver: mpsc::Receiver<CommandEvent>, child: Box<dyn ManagedChild>) -> Result<u64> {
        let mut state = self.state.lock();
        if state.child.is_some() {
            bail!("TrafficTracer Worker is already running");
        }

        let instance_id = self.next_instance_id.fetch_add(1, Ordering::Relaxed);
        let pid = child.pid();
        state.child = Some(RunningChild { instance_id, child });
        drop(state);

        logging!(
            info,
            Type::System,
            "Started TrafficTracer Worker instance {} (PID {})",
            instance_id,
            pid
        );
        Self::watch_events(Arc::clone(&self.state), self.events.clone(), instance_id, receiver);
        Ok(instance_id)
    }

    fn watch_events(
        state: Arc<Mutex<ProcessState>>,
        events: broadcast::Sender<WorkerEvent>,
        instance_id: u64,
        mut receiver: mpsc::Receiver<CommandEvent>,
    ) {
        tauri::async_runtime::spawn(async move {
            let mut terminated = false;
            while let Some(event) = receiver.recv().await {
                match event {
                    CommandEvent::Stdout(bytes) => match String::from_utf8(bytes) {
                        Ok(line) => {
                            let _ = events.send(WorkerEvent::Stdout { instance_id, line });
                        }
                        Err(error) => {
                            let error = error.to_string();
                            logging!(
                                warn,
                                Type::System,
                                "TrafficTracer Worker emitted invalid UTF-8 on stdout: {}",
                                error
                            );
                            let _ = events.send(WorkerEvent::MalformedStdout { instance_id, error });
                        }
                    },
                    CommandEvent::Stderr(bytes) => {
                        let line = String::from_utf8_lossy(&bytes).into_owned();
                        logging!(warn, Type::System, "TrafficTracer Worker stderr: {}", line);
                        let _ = events.send(WorkerEvent::Stderr { instance_id, line });
                    }
                    CommandEvent::Error(error) => {
                        logging!(error, Type::System, "TrafficTracer Worker transport error: {}", error);
                        let _ = events.send(WorkerEvent::TransportError { instance_id, error });
                    }
                    CommandEvent::Terminated(payload) => {
                        terminated = true;
                        Self::finish_instance(&state, instance_id);
                        let _ = events.send(WorkerEvent::Exited {
                            instance_id,
                            status: WorkerExit::from(payload),
                        });
                        break;
                    }
                    _ => {}
                }
            }

            if !terminated {
                Self::finish_instance(&state, instance_id);
                let _ = events.send(WorkerEvent::Exited {
                    instance_id,
                    status: WorkerExit {
                        code: None,
                        signal: None,
                    },
                });
            }
        });
    }

    fn finish_instance(state: &Mutex<ProcessState>, instance_id: u64) {
        let mut state = state.lock();
        if state
            .child
            .as_ref()
            .is_some_and(|running| running.instance_id == instance_id)
        {
            state.child = None;
        }
    }
}

impl From<TerminatedPayload> for WorkerExit {
    fn from(payload: TerminatedPayload) -> Self {
        Self {
            code: payload.code,
            signal: payload.signal,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    struct FakeChild {
        pid: u32,
        kill_count: Arc<AtomicUsize>,
    }

    impl ManagedChild for FakeChild {
        fn pid(&self) -> u32 {
            self.pid
        }

        fn write(&mut self, _bytes: &[u8]) -> Result<()> {
            Ok(())
        }

        fn kill(self: Box<Self>) -> Result<()> {
            self.kill_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn fake_child(pid: u32, kill_count: Arc<AtomicUsize>) -> Box<dyn ManagedChild> {
        Box::new(FakeChild { pid, kill_count })
    }

    #[tokio::test]
    async fn enforces_one_instance_and_stop_is_idempotent() {
        let process = WorkerProcess::new();
        let kill_count = Arc::new(AtomicUsize::new(0));
        let (_sender, receiver) = mpsc::channel(4);

        process
            .attach(receiver, fake_child(42, Arc::clone(&kill_count)))
            .unwrap();
        let (_other_sender, other_receiver) = mpsc::channel(4);
        assert!(
            process
                .attach(other_receiver, fake_child(43, Arc::clone(&kill_count)))
                .is_err()
        );

        assert!(process.stop().unwrap());
        assert!(!process.stop().unwrap());
        assert_eq!(kill_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn reports_exit_and_releases_the_instance() {
        let process = WorkerProcess::new();
        let kill_count = Arc::new(AtomicUsize::new(0));
        let mut events = process.subscribe();
        let (sender, receiver) = mpsc::channel(4);
        let instance_id = process.attach(receiver, fake_child(42, kill_count)).unwrap();

        sender
            .send(CommandEvent::Terminated(TerminatedPayload {
                code: Some(7),
                signal: None,
            }))
            .await
            .unwrap();

        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
                .await
                .unwrap()
                .unwrap(),
            WorkerEvent::Exited {
                instance_id,
                status: WorkerExit {
                    code: Some(7),
                    signal: None,
                },
            }
        );
        assert!(!process.is_running());
    }

    #[tokio::test]
    async fn isolates_malformed_stdout_without_stopping_the_worker() {
        let process = WorkerProcess::new();
        let kill_count = Arc::new(AtomicUsize::new(0));
        let mut events = process.subscribe();
        let (sender, receiver) = mpsc::channel(4);
        let instance_id = process.attach(receiver, fake_child(42, kill_count)).unwrap();

        sender.send(CommandEvent::Stdout(vec![0xff, 0xfe])).await.unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            WorkerEvent::MalformedStdout {
                instance_id: id,
                ..
            } if id == instance_id
        ));
        assert!(process.is_running());
        process.stop().unwrap();
    }
}
