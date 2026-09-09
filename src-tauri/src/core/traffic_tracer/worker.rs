use std::{
    path::Path,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context as _, Result, bail};
use clash_verge_logging::Type;
use parking_lot::Mutex;
use tauri::AppHandle;
use tauri_plugin_shell::{
    ShellExt as _,
    process::{CommandChild, CommandEvent, TerminatedPayload},
};
use tokio::sync::{Semaphore, broadcast, mpsc};

use super::events::TauriEventBridge;
use crate::logging;

const WORKER_SIDECAR_NAME: &str = "traffictracer-worker";
const EVENT_BUFFER_SIZE: usize = 64;
// A stuck native emit must not occupy an async runtime thread, nor create a
// new blocked thread on every Worker restart. The slot is process-wide.
static FRONTEND_EMIT_SLOT: OnceLock<Arc<Semaphore>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerEvent {
    Stdout {
        instance_id: u64,
        line: String,
        journaled: bool,
    },
    MalformedStdout {
        instance_id: u64,
        error: String,
    },
    Stderr {
        instance_id: u64,
        line: String,
    },
    TransportError {
        instance_id: u64,
        error: String,
    },
    Exited {
        instance_id: u64,
        status: WorkerExit,
    },
}

#[doc(hidden)]
pub trait ManagedChild: Send {
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
    pid: u32,
    child: Arc<Mutex<Option<Box<dyn ManagedChild>>>>,
}

#[derive(Default)]
struct ProcessState {
    child: Option<RunningChild>,
    pending_exit: Option<u64>,
    starting: bool,
}

pub struct WorkerProcess {
    journal: Arc<Mutex<Option<std::fs::File>>>,
    state: Arc<Mutex<ProcessState>>,
    next_instance_id: AtomicU64,
    events: broadcast::Sender<WorkerEvent>,
}

impl Default for WorkerProcess {
    fn default() -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER_SIZE);
        Self {
            journal: Arc::new(Mutex::new(None)),
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

    pub fn start(
        &self,
        app_handle: &AppHandle,
        output_root: &Path,
        controller_endpoint: &str,
        controller_secret: &str,
    ) -> Result<u64> {
        {
            let mut state = self.state.lock();
            if state.starting || state.child.is_some() || state.pending_exit.is_some() {
                bail!("TrafficTracer Worker is already running");
            }
            state.starting = true;
        }
        // Native spawning must not hold the state lock used by UI status reads.
        let spawned = (|| {
            self.configure_notification_journal(output_root)?;
            app_handle
                .shell()
                .sidecar(WORKER_SIDECAR_NAME)
                .context("failed to resolve TrafficTracer Worker sidecar")?
                .args([
                    "--output-root",
                    &output_root.to_string_lossy(),
                    "--controller-endpoint",
                    controller_endpoint,
                ])
                .env("TRAFFICTRACER_CONTROLLER_SECRET", controller_secret)
                .spawn()
                .context("failed to start TrafficTracer Worker sidecar")
        })();
        let mut state = self.state.lock();
        state.starting = false;
        let (receiver, child) = spawned?;
        let instance_id = self.next_instance_id.fetch_add(1, Ordering::Relaxed);
        let pid = child.pid();
        state.child = Some(RunningChild {
            instance_id,
            pid,
            child: Arc::new(Mutex::new(Some(Box::new(child)))),
        });
        drop(state);

        logging!(
            info,
            Type::System,
            "Started TrafficTracer Worker instance {} (PID {})",
            instance_id,
            pid
        );
        Self::watch_events(
            Arc::clone(&self.state),
            self.events.clone(),
            instance_id,
            receiver,
            self.journal.clone(),
        );
        Ok(instance_id)
    }

    pub fn stop(&self) -> Result<bool> {
        let running = {
            let mut state = self.state.lock();
            let running = state.child.take();
            if let Some(running) = &running {
                state.pending_exit = Some(running.instance_id);
            }
            running
        };
        let Some(running) = running else {
            return Ok(false);
        };

        logging!(
            info,
            Type::System,
            "Stopping TrafficTracer Worker instance {} (PID {})",
            running.instance_id,
            running.pid
        );
        if let Some(child) = running.child.lock().take() {
            child.kill()?;
        }
        Ok(true)
    }

    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        let child = {
            let state = self.state.lock();
            Arc::clone(
                &state
                    .child
                    .as_ref()
                    .context("TrafficTracer Worker is not running")?
                    .child,
            )
        };
        // Pipe backpressure must never block process-state queries or exit routing.
        let mut child = child.lock();
        child.as_mut().context("TrafficTracer Worker is stopping")?.write(bytes)
    }

    pub fn is_running(&self) -> bool {
        let state = self.state.lock();
        state.starting || state.child.is_some() || state.pending_exit.is_some()
    }

    pub fn instance_id(&self) -> Option<u64> {
        let state = self.state.lock();
        state
            .child
            .as_ref()
            .map(|running| running.instance_id)
            .or(state.pending_exit)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WorkerEvent> {
        self.events.subscribe()
    }

    pub fn bridge_to_tauri(&self, app_handle: AppHandle) -> tauri::async_runtime::JoinHandle<()> {
        let mut receiver = self.subscribe();
        let bridge = Arc::new(TauriEventBridge::new(app_handle));
        let slot = FRONTEND_EMIT_SLOT.get_or_init(|| Arc::new(Semaphore::new(1))).clone();
        tauri::async_runtime::spawn(async move {
            let mut flush = tokio::time::interval(Duration::from_millis(200));
            flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let event = tokio::select! {
                    event = receiver.recv() => event,
                    _ = flush.tick() => {
                        let Ok(permit) = Arc::clone(&slot).acquire_owned().await else { break; };
                        let bridge = Arc::clone(&bridge);
                        let _ = tauri::async_runtime::spawn_blocking(move || {
                            let _permit = permit;
                            bridge.flush_progress();
                        }).await;
                        continue;
                    }
                };
                match event {
                    Ok(WorkerEvent::Stdout { line, journaled, .. }) => {
                        let Ok(permit) = Arc::clone(&slot).acquire_owned().await else {
                            break;
                        };
                        let bridge = Arc::clone(&bridge);
                        let _ = tauri::async_runtime::spawn_blocking(move || {
                            let _permit = permit;
                            bridge.handle_line(&line, journaled);
                        })
                        .await;
                    }
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

    /// Attaches an already spawned transport. This is intentionally hidden from
    /// application consumers and exists so integration tests can exercise the
    /// complete Worker orchestration without a packaged sidecar.
    #[doc(hidden)]
    pub fn attach(&self, receiver: mpsc::Receiver<CommandEvent>, child: Box<dyn ManagedChild>) -> Result<u64> {
        let mut state = self.state.lock();
        if state.starting || state.child.is_some() || state.pending_exit.is_some() {
            bail!("TrafficTracer Worker is already running");
        }

        let instance_id = self.next_instance_id.fetch_add(1, Ordering::Relaxed);
        let pid = child.pid();
        state.child = Some(RunningChild {
            instance_id,
            pid,
            child: Arc::new(Mutex::new(Some(child))),
        });
        drop(state);

        logging!(
            info,
            Type::System,
            "Started TrafficTracer Worker instance {} (PID {})",
            instance_id,
            pid
        );
        Self::watch_events(
            Arc::clone(&self.state),
            self.events.clone(),
            instance_id,
            receiver,
            self.journal.clone(),
        );
        Ok(instance_id)
    }

    fn watch_events(
        state: Arc<Mutex<ProcessState>>,
        events: broadcast::Sender<WorkerEvent>,
        instance_id: u64,
        mut receiver: mpsc::Receiver<CommandEvent>,
        journal: Arc<Mutex<Option<std::fs::File>>>,
    ) {
        tauri::async_runtime::spawn(async move {
            let mut terminated = false;
            while let Some(event) = receiver.recv().await {
                match event {
                    CommandEvent::Stdout(bytes) => match String::from_utf8(bytes) {
                        Ok(line) => {
                            let journaled = if super::notification_delivery::needs_journal(&line) {
                                let journal = journal.clone();
                                let saved_line = line.clone();
                                tauri::async_runtime::spawn_blocking(move || {
                                    let mut guard = journal.lock();
                                    match guard.as_mut() {
                                        Some(file) => match super::notification_delivery::append(file, &saved_line) {
                                            Ok(()) => true,
                                            Err(error) => {
                                                super::notification_delivery::global().lock().journal_failures += 1;
                                                logging!(error, Type::System, "TrafficTracer notification journal failed: {}; unsaved notification: {}", error, saved_line);
                                                false
                                            }
                                        },
                                        None => false,
                                    }
                                }).await.unwrap_or(false)
                            } else {
                                false
                            };
                            let _ = events.send(WorkerEvent::Stdout {
                                instance_id,
                                line,
                                journaled,
                            });
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
                        logging!(
                            info,
                            Type::System,
                            "TrafficTracer Worker instance {} exited: code {:?}, signal {:?}",
                            instance_id,
                            payload.code,
                            payload.signal
                        );
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

    pub fn configure_notification_journal(&self, root: &Path) -> Result<()> {
        *self.journal.lock() = Some(super::notification_delivery::open_journal(root)?);
        Ok(())
    }

    fn finish_instance(state: &Mutex<ProcessState>, instance_id: u64) {
        let mut state = state.lock();
        if state.pending_exit == Some(instance_id) {
            state.pending_exit = None;
        }
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
    async fn journals_notifications_before_broadcast_even_when_a_subscriber_lags() {
        let root = std::env::temp_dir().join(format!(
            "tt-prebroadcast-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_micros()
        ));
        let process = WorkerProcess::new();
        process.configure_notification_journal(&root).unwrap();
        let mut observer = process.subscribe();
        let (sender, receiver) = mpsc::channel(4);
        process
            .attach(receiver, fake_child(42, Arc::new(AtomicUsize::new(0))))
            .unwrap();
        for id in 0..100 {
            let line = serde_json::json!({"type":"notification","method":"worker.log","params":{"id":id}}).to_string();
            sender.send(CommandEvent::Stdout(line.into_bytes())).await.unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                match observer.recv().await {
                    Ok(WorkerEvent::Stdout { line, journaled, .. }) if line.contains("\"id\":99") => {
                        assert!(journaled);
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => panic!("closed before journal completion"),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        let path = std::fs::read_dir(root.join("diagnostics/worker-notifications"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(text.lines().count(), 100);
        assert!(text.contains("\"id\":0"));
        process.stop().unwrap();
        drop(sender);
        std::fs::remove_dir_all(root).unwrap();
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
        assert!(process.is_running(), "kill request is not exit confirmation");
        let (_sender, receiver) = mpsc::channel(4);
        assert!(process.attach(receiver, fake_child(44, kill_count)).is_err());
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
