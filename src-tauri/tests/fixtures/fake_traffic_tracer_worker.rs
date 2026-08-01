use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::Result;
use app_lib::traffic_tracer_test_support::{
    FLOW_SCHEMA_VERSION, JOB_SCHEMA_VERSION, ManagedChild, SESSION_SCHEMA_VERSION, WORKER_API_VERSION,
};
use serde_json::{Value, json};
use tauri_plugin_shell::process::{CommandEvent, TerminatedPayload};
use tokio::sync::mpsc;

#[derive(Clone, Default)]
pub struct FakeWorkerProbe {
    writes: Arc<Mutex<Vec<Value>>>,
    kills: Arc<AtomicUsize>,
}

impl FakeWorkerProbe {
    pub fn writes(&self) -> Vec<Value> {
        self.writes.lock().unwrap().clone()
    }

    pub fn kill_count(&self) -> usize {
        self.kills.load(Ordering::SeqCst)
    }
}

pub struct FakeTrafficTracerWorker {
    events: mpsc::Sender<CommandEvent>,
    probe: FakeWorkerProbe,
    held_echo: Option<Value>,
}

impl FakeTrafficTracerWorker {
    pub fn new(events: mpsc::Sender<CommandEvent>) -> (Self, FakeWorkerProbe) {
        let probe = FakeWorkerProbe::default();
        (
            Self {
                events,
                probe: probe.clone(),
                held_echo: None,
            },
            probe,
        )
    }

    fn emit(&self, value: Value) -> Result<()> {
        self.events
            .try_send(CommandEvent::Stdout(serde_json::to_vec(&value)?))?;
        Ok(())
    }

    fn respond(&self, id: Value, result: Value) -> Result<()> {
        self.emit(json!({
            "api_version": WORKER_API_VERSION,
            "type": "response",
            "id": id,
            "result": result,
        }))
    }

    fn handle(&mut self, request: Value) -> Result<()> {
        let id = request["id"].clone();
        match request["method"].as_str().unwrap_or_default() {
            "hello" => self.respond(
                id,
                json!({
                    "product": "TrafficTracer Complete Fake Worker",
                    "version": "test",
                    "api_version": WORKER_API_VERSION,
                    "job_schema_version": JOB_SCHEMA_VERSION,
                    "session_schema_version": SESSION_SCHEMA_VERSION,
                    "flow_schema_version": FLOW_SCHEMA_VERSION,
                    "methods": [
                        "hello", "environment.diagnose", "job.start",
                        "job.cancel", "job.status", "worker.shutdown"
                    ],
                }),
            ),
            "environment.diagnose" => {
                let response = json!({
                    "api_version": WORKER_API_VERSION,
                    "type": "response",
                    "id": id,
                    "result": { "echo": request["params"]["echo"].clone() },
                });
                if let Some(held) = self.held_echo.take() {
                    self.emit(response)?;
                    self.emit(held)
                } else {
                    self.held_echo = Some(response);
                    Ok(())
                }
            }
            "job.start" if request["params"]["return_error"] == true => self.emit(json!({
                "api_version": WORKER_API_VERSION,
                "type": "response",
                "id": id,
                "error": {
                    "code": "JOB_BUSY",
                    "message": "another Job is active",
                    "data": { "active_job_id": "job-existing" }
                }
            })),
            "job.start" => {
                let job_id = request["params"]["job_id"].clone();
                self.emit(json!({
                    "api_version": WORKER_API_VERSION,
                    "type": "notification",
                    "method": "job.progress",
                    "params": {
                        "job_id": job_id,
                        "stage": "capturing",
                        "progress": 0.5
                    }
                }))?;
                self.respond(id, json!({ "job_id": job_id, "state": "capturing" }))
            }
            "job.cancel" => self.respond(
                id,
                json!({
                    "job_id": request["params"]["job_id"].clone(),
                    "state": "cancelled"
                }),
            ),
            "job.status" if request["params"]["crash"] == true => {
                self.events.try_send(CommandEvent::Terminated(TerminatedPayload {
                    code: Some(17),
                    signal: None,
                }))?;
                Ok(())
            }
            "worker.shutdown" => {
                self.respond(id, json!({ "shutdown": true, "jobs_stopped": true }))?;
                self.events.try_send(CommandEvent::Terminated(TerminatedPayload {
                    code: Some(0),
                    signal: None,
                }))?;
                Ok(())
            }
            method => self.emit(json!({
                "api_version": WORKER_API_VERSION,
                "type": "response",
                "id": id,
                "error": {
                    "code": "METHOD_NOT_FOUND",
                    "message": format!("unsupported fake method: {method}")
                }
            })),
        }
    }
}

impl ManagedChild for FakeTrafficTracerWorker {
    fn pid(&self) -> u32 {
        4242
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let request: Value = serde_json::from_slice(bytes)?;
        self.probe.writes.lock().unwrap().push(request.clone());
        self.handle(request)
    }

    fn kill(self: Box<Self>) -> Result<()> {
        self.probe.kills.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
