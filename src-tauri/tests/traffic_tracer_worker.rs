#[path = "fixtures/fake_traffic_tracer_worker.rs"]
mod fake_traffic_tracer_worker;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use app_lib::traffic_tracer_test_support::{
    ClientError, EVENT_JOB_PROGRESS, EmptyParams, HandshakeState, NotificationMapper, RequestMethod, WorkerClient,
    WorkerErrorCode, WorkerEvent, WorkerProcess,
};
use fake_traffic_tracer_worker::{FakeTrafficTracerWorker, FakeWorkerProbe};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri_plugin_shell::process::CommandEvent;
use tokio::sync::{broadcast, mpsc};

struct Harness {
    process: Arc<WorkerProcess>,
    client: Arc<WorkerClient>,
    events: broadcast::Receiver<WorkerEvent>,
    probe: FakeWorkerProbe,
}

impl Harness {
    fn new() -> Self {
        let process = Arc::new(WorkerProcess::new());
        let events = process.subscribe();
        let (event_sender, event_receiver) = mpsc::channel::<CommandEvent>(32);
        let (worker, probe) = FakeTrafficTracerWorker::new(event_sender);
        process.attach(event_receiver, Box::new(worker)).unwrap();
        let client = Arc::new(WorkerClient::new(Arc::clone(&process), Duration::from_secs(1)));
        Self {
            process,
            client,
            events,
            probe,
        }
    }

    fn stop(&self) {
        if self.process.is_running() {
            self.process.stop().unwrap();
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct EchoResult {
    echo: String,
}

#[derive(Debug, Serialize)]
struct EchoParams<'a> {
    echo: &'a str,
}

#[derive(Debug, Deserialize)]
struct JobResult {
    job_id: String,
    state: String,
}

async fn wait_until_stopped(process: &WorkerProcess) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while process.is_running() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn traffic_tracer_negotiates_hello_with_the_fake_worker() {
    let harness = Harness::new();
    let hello = harness.client.hello().await.unwrap();

    assert_eq!(hello.product, "TrafficTracer Complete Fake Worker");
    assert!(matches!(
        harness.client.handshake_state(),
        HandshakeState::Ready(ref ready) if ready == &hello
    ));
    harness.stop();
}

#[tokio::test]
async fn traffic_tracer_correlates_responses_that_arrive_out_of_order() {
    let harness = Harness::new();
    let first_client = Arc::clone(&harness.client);
    let second_client = Arc::clone(&harness.client);
    let first = tokio::spawn(async move {
        first_client
            .request::<_, EchoResult>(RequestMethod::EnvironmentDiagnose, EchoParams { echo: "first" })
            .await
    });
    let second = tokio::spawn(async move {
        second_client
            .request::<_, EchoResult>(RequestMethod::EnvironmentDiagnose, EchoParams { echo: "second" })
            .await
    });

    assert_eq!(first.await.unwrap().unwrap().echo, "first");
    assert_eq!(second.await.unwrap().unwrap().echo, "second");
    harness.stop();
}

#[tokio::test]
async fn traffic_tracer_forwards_progress_without_confusing_it_for_a_response() {
    let mut harness = Harness::new();
    let result: JobResult = harness
        .client
        .request(RequestMethod::JobStart, serde_json::json!({ "job_id": "job-progress" }))
        .await
        .unwrap();
    assert_eq!(result.job_id, "job-progress");
    assert_eq!(result.state, "capturing");

    let line = loop {
        match tokio::time::timeout(Duration::from_secs(1), harness.events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            WorkerEvent::Stdout { line, .. }
                if serde_json::from_str::<Value>(&line).unwrap()["type"] == "notification" =>
            {
                break line;
            }
            _ => {}
        }
    };
    let event = NotificationMapper::default()
        .map_line(&line, Instant::now())
        .unwrap()
        .unwrap();
    assert_eq!(event.name, EVENT_JOB_PROGRESS);
    assert_eq!(event.payload["job_id"], "job-progress");
    assert_eq!(event.payload["progress"], 0.5);
    harness.stop();
}

#[tokio::test]
async fn traffic_tracer_sends_cancel_and_maps_worker_errors() {
    let harness = Harness::new();
    let cancelled: JobResult = harness
        .client
        .request(RequestMethod::JobCancel, serde_json::json!({ "job_id": "job-cancel" }))
        .await
        .unwrap();
    assert_eq!(cancelled.state, "cancelled");

    let error = harness
        .client
        .request::<_, Value>(RequestMethod::JobStart, serde_json::json!({ "return_error": true }))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ClientError::Worker {
            code: WorkerErrorCode::JobBusy,
            ref message,
            data: Some(ref data),
        } if message == "another Job is active" && data["active_job_id"] == "job-existing"
    ));
    assert_eq!(harness.probe.writes()[0]["method"], "job.cancel");
    harness.stop();
}

#[tokio::test]
async fn traffic_tracer_fails_pending_requests_when_the_worker_crashes() {
    let harness = Harness::new();
    let error = harness
        .client
        .request::<_, Value>(RequestMethod::JobStatus, serde_json::json!({ "crash": true }))
        .await
        .unwrap_err();

    assert_eq!(error, ClientError::WorkerExited);
    wait_until_stopped(&harness.process).await;
    assert_eq!(harness.probe.kill_count(), 0);
}

#[tokio::test]
async fn traffic_tracer_acknowledges_shutdown_and_observes_clean_exit() {
    #[derive(Deserialize)]
    struct ShutdownResult {
        shutdown: bool,
        jobs_stopped: bool,
    }

    let harness = Harness::new();
    let result: ShutdownResult = harness
        .client
        .request(RequestMethod::WorkerShutdown, EmptyParams::default())
        .await
        .unwrap();

    assert!(result.shutdown);
    assert!(result.jobs_stopped);
    wait_until_stopped(&harness.process).await;
    assert_eq!(harness.probe.kill_count(), 0);
}
