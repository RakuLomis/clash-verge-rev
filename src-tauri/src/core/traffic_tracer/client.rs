use std::{
    collections::{HashMap, hash_map::Entry},
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::sync::{broadcast, oneshot};

use super::{
    protocol::{
        EmptyParams, FLOW_SCHEMA_VERSION, JOB_SCHEMA_VERSION, NotificationMethod, Request, RequestId, RequestMethod,
        SESSION_SCHEMA_VERSION, WORKER_API_VERSION, WorkerError, WorkerErrorCode,
    },
    worker::{WorkerEvent, WorkerProcess},
};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

type PendingResult = Result<Value, ClientError>;
type PendingSender = oneshot::Sender<PendingResult>;

#[derive(Clone, Debug, PartialEq)]
pub enum ClientError {
    DuplicateRequestId(RequestId),
    Encode(String),
    Decode(String),
    Protocol(String),
    Transport(String),
    Timeout(RequestId),
    VersionMismatch {
        component: &'static str,
        supported: u32,
        actual: u32,
    },
    WorkerExited,
    Worker {
        code: WorkerErrorCode,
        message: String,
        data: Option<Value>,
    },
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateRequestId(id) => write!(formatter, "duplicate Worker request ID: {id:?}"),
            Self::Encode(message) => write!(formatter, "failed to encode Worker request: {message}"),
            Self::Decode(message) => write!(formatter, "failed to decode Worker response: {message}"),
            Self::Protocol(message) => write!(formatter, "invalid Worker response: {message}"),
            Self::Transport(message) => write!(formatter, "Worker transport failed: {message}"),
            Self::Timeout(id) => write!(formatter, "Worker request timed out: {id:?}"),
            Self::VersionMismatch {
                component,
                supported,
                actual,
            } => write!(
                formatter,
                "incompatible {component} version: application supports {supported}, Worker reported {actual}"
            ),
            Self::WorkerExited => write!(formatter, "TrafficTracer Worker exited"),
            Self::Worker { code, message, .. } => {
                write!(formatter, "Worker returned {code:?}: {message}")
            }
        }
    }
}

impl std::error::Error for ClientError {}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct HelloResult {
    pub product: String,
    pub version: String,
    pub api_version: u32,
    pub job_schema_version: u32,
    pub session_schema_version: u32,
    pub flow_schema_version: u32,
    pub methods: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HandshakeState {
    NotStarted,
    Negotiating,
    Ready(HelloResult),
    Failed(ClientError),
}

pub struct WorkerClient {
    process: Arc<WorkerProcess>,
    pending: Arc<Mutex<HashMap<RequestId, PendingSender>>>,
    next_request_id: AtomicU64,
    request_timeout: Duration,
    handshake: Arc<Mutex<HandshakeState>>,
    router: tauri::async_runtime::JoinHandle<()>,
}

impl WorkerClient {
    pub fn new(process: Arc<WorkerProcess>, request_timeout: Duration) -> Self {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let handshake = Arc::new(Mutex::new(HandshakeState::NotStarted));
        let router = Self::spawn_router(process.subscribe(), Arc::clone(&pending), Arc::clone(&handshake));

        Self {
            process,
            pending,
            next_request_id: AtomicU64::new(1),
            request_timeout,
            handshake,
            router,
        }
    }

    pub fn handshake_state(&self) -> HandshakeState {
        self.handshake.lock().clone()
    }

    pub async fn hello(&self) -> Result<HelloResult, ClientError> {
        {
            let mut state = self.handshake.lock();
            match &*state {
                HandshakeState::Ready(result) => return Ok(result.clone()),
                HandshakeState::Negotiating => {
                    return Err(ClientError::Protocol("Worker hello is already in progress".to_owned()));
                }
                HandshakeState::NotStarted | HandshakeState::Failed(_) => {
                    *state = HandshakeState::Negotiating;
                }
            }
        }

        let result = self
            .request::<_, HelloResult>(RequestMethod::Hello, EmptyParams::default())
            .await
            .and_then(Self::validate_hello);
        *self.handshake.lock() = match &result {
            Ok(hello) => HandshakeState::Ready(hello.clone()),
            Err(error) => HandshakeState::Failed(error.clone()),
        };
        result
    }

    fn validate_hello(hello: HelloResult) -> Result<HelloResult, ClientError> {
        for (component, supported, actual) in [
            ("Worker API", WORKER_API_VERSION, hello.api_version),
            ("Job schema", JOB_SCHEMA_VERSION, hello.job_schema_version),
            ("Session schema", SESSION_SCHEMA_VERSION, hello.session_schema_version),
            ("Flow schema", FLOW_SCHEMA_VERSION, hello.flow_schema_version),
        ] {
            if actual != supported {
                return Err(ClientError::VersionMismatch {
                    component,
                    supported,
                    actual,
                });
            }
        }
        Ok(hello)
    }

    pub async fn request<P, R>(&self, method: RequestMethod, params: P) -> Result<R, ClientError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let id = RequestId::Integer(self.next_request_id.fetch_add(1, Ordering::Relaxed));
        let request = Request::new(id.clone(), method, params);
        let mut encoded = serde_json::to_vec(&request).map_err(|error| ClientError::Encode(error.to_string()))?;
        if encoded.len() > MAX_REQUEST_BYTES {
            return Err(ClientError::Encode(format!(
                "request exceeds {MAX_REQUEST_BYTES} bytes"
            )));
        }
        encoded.push(b'\n');

        let (sender, receiver) = oneshot::channel();
        self.register_pending(id.clone(), sender)?;
        if let Err(error) = self.process.write(&encoded) {
            self.pending.lock().remove(&id);
            return Err(ClientError::Transport(error.to_string()));
        }

        let result = match tokio::time::timeout(self.request_timeout, receiver).await {
            Ok(Ok(result)) => result?,
            Ok(Err(_)) => return Err(ClientError::WorkerExited),
            Err(_) => {
                self.pending.lock().remove(&id);
                return Err(ClientError::Timeout(id));
            }
        };

        serde_json::from_value(result).map_err(|error| ClientError::Decode(error.to_string()))
    }

    fn register_pending(&self, id: RequestId, sender: PendingSender) -> Result<(), ClientError> {
        match self.pending.lock().entry(id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(sender);
                Ok(())
            }
            Entry::Occupied(_) => Err(ClientError::DuplicateRequestId(id)),
        }
    }

    fn spawn_router(
        mut events: broadcast::Receiver<WorkerEvent>,
        pending: Arc<Mutex<HashMap<RequestId, PendingSender>>>,
        handshake: Arc<Mutex<HandshakeState>>,
    ) -> tauri::async_runtime::JoinHandle<()> {
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(WorkerEvent::Stdout { line, .. }) => {
                        Self::route_line(&pending, &line);
                    }
                    Ok(WorkerEvent::TransportError { error, .. }) => {
                        let error = ClientError::Transport(error);
                        *handshake.lock() = HandshakeState::Failed(error.clone());
                        Self::fail_all(&pending, error);
                    }
                    Ok(WorkerEvent::Exited { .. }) => {
                        *handshake.lock() = HandshakeState::Failed(ClientError::WorkerExited);
                        Self::fail_all(&pending, ClientError::WorkerExited);
                    }
                    Ok(WorkerEvent::MalformedStdout { .. } | WorkerEvent::Stderr { .. }) => {}
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        let error = ClientError::Transport(format!("missed {count} Worker process events"));
                        *handshake.lock() = HandshakeState::Failed(error.clone());
                        Self::fail_all(&pending, error);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        *handshake.lock() = HandshakeState::Failed(ClientError::WorkerExited);
                        Self::fail_all(&pending, ClientError::WorkerExited);
                        break;
                    }
                }
            }
        })
    }

    fn route_line(pending: &Mutex<HashMap<RequestId, PendingSender>>, line: &str) {
        let message = match serde_json::from_str::<InboundMessage>(line) {
            Ok(message) => message,
            Err(_) => return,
        };

        let InboundMessage::Response {
            api_version,
            id,
            result,
            error,
        } = message
        else {
            return;
        };
        let Some(id) = id else {
            return;
        };
        let Some(sender) = pending.lock().remove(&id) else {
            return;
        };

        let response = if api_version != WORKER_API_VERSION {
            Err(ClientError::Protocol(format!(
                "response API version {api_version} does not match {WORKER_API_VERSION}"
            )))
        } else {
            match (result, error) {
                (Some(result), None) => Ok(result),
                (None, Some(error)) => Err(ClientError::Worker {
                    code: error.code,
                    message: error.message,
                    data: error.data,
                }),
                (Some(_), Some(_)) => Err(ClientError::Protocol(
                    "response contains both result and error".to_owned(),
                )),
                (None, None) => Err(ClientError::Protocol(
                    "response contains neither result nor error".to_owned(),
                )),
            }
        };
        let _ = sender.send(response);
    }

    fn fail_all(pending: &Mutex<HashMap<RequestId, PendingSender>>, error: ClientError) {
        for (_, sender) in pending.lock().drain() {
            let _ = sender.send(Err(error.clone()));
        }
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        self.router.abort();
        Self::fail_all(
            &self.pending,
            ClientError::Transport("Worker client was dropped".to_owned()),
        );
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InboundMessage {
    Response {
        api_version: u32,
        id: Option<RequestId>,
        #[serde(default)]
        result: Option<Value>,
        #[serde(default)]
        error: Option<WorkerError<Value>>,
    },
    Notification {
        api_version: u32,
        method: NotificationMethod,
        params: Value,
    },
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use anyhow::Result;
    use serde::Deserialize;
    use tauri_plugin_shell::process::CommandEvent;
    use tokio::sync::mpsc;

    use super::*;
    use crate::core::traffic_tracer::{protocol::EmptyParams, worker::ManagedChild};

    struct FakeChild {
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
        kills: Arc<AtomicUsize>,
    }

    impl ManagedChild for FakeChild {
        fn pid(&self) -> u32 {
            42
        }

        fn write(&mut self, bytes: &[u8]) -> Result<()> {
            self.writes.lock().push(bytes.to_vec());
            Ok(())
        }

        fn kill(self: Box<Self>) -> Result<()> {
            self.kills.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct Harness {
        process: Arc<WorkerProcess>,
        client: Arc<WorkerClient>,
        events: mpsc::Sender<CommandEvent>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    fn harness(timeout: Duration) -> Harness {
        let process = Arc::new(WorkerProcess::new());
        let writes = Arc::new(Mutex::new(Vec::new()));
        let kills = Arc::new(AtomicUsize::new(0));
        let (events, receiver) = mpsc::channel(16);
        process
            .attach(
                receiver,
                Box::new(FakeChild {
                    writes: Arc::clone(&writes),
                    kills,
                }),
            )
            .unwrap();
        let client = Arc::new(WorkerClient::new(Arc::clone(&process), timeout));
        Harness {
            process,
            client,
            events,
            writes,
        }
    }

    async fn wait_for_writes(writes: &Mutex<Vec<Vec<u8>>>, count: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while writes.lock().len() < count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    fn response(id: RequestId, value: &str) -> CommandEvent {
        CommandEvent::Stdout(
            serde_json::to_vec(&serde_json::json!({
                "api_version": WORKER_API_VERSION,
                "type": "response",
                "id": id,
                "result": {"value": value}
            }))
            .unwrap(),
        )
    }

    #[derive(Deserialize, Debug, PartialEq, Eq)]
    struct Reply {
        value: String,
    }

    fn valid_hello() -> Value {
        serde_json::json!({
            "product": "TrafficTracer Complete Worker",
            "version": "0.1.0",
            "api_version": WORKER_API_VERSION,
            "job_schema_version": JOB_SCHEMA_VERSION,
            "session_schema_version": SESSION_SCHEMA_VERSION,
            "flow_schema_version": FLOW_SCHEMA_VERSION,
            "methods": ["hello", "environment.diagnose"]
        })
    }

    async fn negotiate(harness: &Harness, hello: Value) -> Result<HelloResult, ClientError> {
        let client = Arc::clone(&harness.client);
        let negotiation = tokio::spawn(async move { client.hello().await });
        wait_for_writes(&harness.writes, 1).await;
        let request: Request<EmptyParams> = serde_json::from_slice(&harness.writes.lock()[0]).unwrap();
        harness
            .events
            .send(CommandEvent::Stdout(
                serde_json::to_vec(&serde_json::json!({
                    "api_version": WORKER_API_VERSION,
                    "type": "response",
                    "id": request.id,
                    "result": hello
                }))
                .unwrap(),
            ))
            .await
            .unwrap();
        negotiation.await.unwrap()
    }

    #[tokio::test]
    async fn hello_marks_matching_worker_ready() {
        let harness = harness(Duration::from_secs(1));
        let hello = negotiate(&harness, valid_hello()).await.unwrap();

        assert_eq!(harness.client.handshake_state(), HandshakeState::Ready(hello));
        harness.process.stop().unwrap();
    }

    #[tokio::test]
    async fn hello_rejects_older_schema() {
        let harness = harness(Duration::from_secs(1));
        let mut hello = valid_hello();
        hello["session_schema_version"] = serde_json::json!(0);
        let error = negotiate(&harness, hello).await.unwrap_err();

        assert!(matches!(
            error,
            ClientError::VersionMismatch {
                component: "Session schema",
                supported: SESSION_SCHEMA_VERSION,
                actual: 0,
            }
        ));
        assert!(matches!(
            harness.client.handshake_state(),
            HandshakeState::Failed(ClientError::VersionMismatch { .. })
        ));
        harness.process.stop().unwrap();
    }

    #[tokio::test]
    async fn hello_rejects_newer_schema() {
        let harness = harness(Duration::from_secs(1));
        let mut hello = valid_hello();
        hello["flow_schema_version"] = serde_json::json!(FLOW_SCHEMA_VERSION + 1);
        let error = negotiate(&harness, hello).await.unwrap_err();

        assert!(matches!(
            error,
            ClientError::VersionMismatch {
                component: "Flow schema",
                supported: FLOW_SCHEMA_VERSION,
                actual,
            } if actual == FLOW_SCHEMA_VERSION + 1
        ));
        assert!(!matches!(harness.client.handshake_state(), HandshakeState::Ready(_)));
        harness.process.stop().unwrap();
    }

    #[tokio::test]
    async fn hello_rejects_missing_version_field() {
        let harness = harness(Duration::from_secs(1));
        let mut hello = valid_hello();
        hello.as_object_mut().unwrap().remove("flow_schema_version");
        let error = negotiate(&harness, hello).await.unwrap_err();

        assert!(matches!(error, ClientError::Decode(_)));
        assert!(matches!(
            harness.client.handshake_state(),
            HandshakeState::Failed(ClientError::Decode(_))
        ));
        harness.process.stop().unwrap();
    }

    #[tokio::test]
    async fn correlates_out_of_order_responses() {
        let harness = harness(Duration::from_secs(1));
        let first_client = Arc::clone(&harness.client);
        let second_client = Arc::clone(&harness.client);
        let first = tokio::spawn(async move {
            first_client
                .request::<_, Reply>(RequestMethod::Hello, EmptyParams::default())
                .await
        });
        let second = tokio::spawn(async move {
            second_client
                .request::<_, Reply>(RequestMethod::Hello, EmptyParams::default())
                .await
        });
        wait_for_writes(&harness.writes, 2).await;

        let requests: Vec<Request<EmptyParams>> = harness
            .writes
            .lock()
            .iter()
            .map(|bytes| serde_json::from_slice(bytes).unwrap())
            .collect();
        harness
            .events
            .send(response(requests[1].id.clone(), "second"))
            .await
            .unwrap();
        harness
            .events
            .send(response(requests[0].id.clone(), "first"))
            .await
            .unwrap();

        assert_eq!(first.await.unwrap().unwrap().value, "first");
        assert_eq!(second.await.unwrap().unwrap().value, "second");
        harness.process.stop().unwrap();
    }

    #[tokio::test]
    async fn rejects_duplicate_pending_ids() {
        let harness = harness(Duration::from_secs(1));
        let id = RequestId::Integer(99);
        let (first, _first_receiver) = oneshot::channel();
        let (second, _second_receiver) = oneshot::channel();
        harness.client.register_pending(id.clone(), first).unwrap();

        assert_eq!(
            harness.client.register_pending(id.clone(), second),
            Err(ClientError::DuplicateRequestId(id))
        );
        harness.process.stop().unwrap();
    }

    #[tokio::test]
    async fn times_out_and_removes_pending_request() {
        let harness = harness(Duration::from_millis(10));
        let error = harness
            .client
            .request::<_, Reply>(RequestMethod::Hello, EmptyParams::default())
            .await
            .unwrap_err();

        assert!(matches!(error, ClientError::Timeout(_)));
        assert!(harness.client.pending.lock().is_empty());
        harness.process.stop().unwrap();
    }

    #[tokio::test]
    async fn eof_fails_all_pending_requests() {
        let Harness {
            process,
            client,
            events,
            writes,
        } = harness(Duration::from_secs(1));
        let request_client = Arc::clone(&client);
        let request = tokio::spawn(async move {
            request_client
                .request::<_, Reply>(RequestMethod::Hello, EmptyParams::default())
                .await
        });
        wait_for_writes(&writes, 1).await;
        drop(events);

        assert_eq!(request.await.unwrap(), Err(ClientError::WorkerExited));
        assert!(client.pending.lock().is_empty());
        assert!(!process.is_running());
    }

    #[tokio::test]
    async fn duplicate_response_does_not_complete_another_request() {
        let harness = harness(Duration::from_millis(25));
        let client = Arc::clone(&harness.client);
        let request = tokio::spawn(async move {
            client
                .request::<_, Reply>(RequestMethod::Hello, EmptyParams::default())
                .await
        });
        wait_for_writes(&harness.writes, 1).await;
        let first: Request<EmptyParams> = serde_json::from_slice(&harness.writes.lock()[0]).unwrap();

        harness.events.send(response(first.id.clone(), "ok")).await.unwrap();
        harness.events.send(response(first.id, "duplicate")).await.unwrap();

        assert_eq!(request.await.unwrap().unwrap().value, "ok");
        assert!(harness.client.pending.lock().is_empty());
        harness.process.stop().unwrap();
    }

    #[test]
    fn inbound_message_type_matches_protocol_tag() {
        let raw = r#"{"type":"notification","api_version":1,"method":"worker.ready","params":{}}"#;
        let InboundMessage::Notification {
            api_version,
            method,
            params,
        } = serde_json::from_str::<InboundMessage>(raw).unwrap()
        else {
            panic!("expected notification");
        };
        assert_eq!(api_version, WORKER_API_VERSION);
        assert_eq!(method, NotificationMethod::WorkerReady);
        assert_eq!(params, serde_json::json!({}));
    }
}
