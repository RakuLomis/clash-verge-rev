use serde::{Deserialize, Serialize};

pub const WORKER_API_VERSION: u32 = 2;
pub const JOB_SCHEMA_VERSION: u32 = 3;
pub const SESSION_SCHEMA_VERSION: u32 = 2;
pub const FLOW_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Text(String),
    Integer(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Request,
    Response,
    Notification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestMethod {
    #[serde(rename = "hello")]
    Hello,
    #[serde(rename = "environment.diagnose")]
    EnvironmentDiagnose,
    #[serde(rename = "config.targets.load")]
    ConfigTargetsLoad,
    #[serde(rename = "job.start")]
    JobStart,
    #[serde(rename = "job.cancel")]
    JobCancel,
    #[serde(rename = "job.interrupt")]
    JobInterrupt,
    #[serde(rename = "job.status")]
    JobStatus,
    #[serde(rename = "analysis.start")]
    AnalysisStart,
    #[serde(rename = "packet_split.start")]
    PacketSplitStart,
    #[serde(rename = "packet_split.resume")]
    PacketSplitResume,
    #[serde(rename = "session.list")]
    SessionList,
    #[serde(rename = "session.scope.resolve")]
    SessionScopeResolve,
    #[serde(rename = "session.scope.list")]
    SessionScopeList,
    #[serde(rename = "session.scope.packet_split.preview")]
    SessionScopePacketSplitPreview,
    #[serde(rename = "session.get")]
    SessionGet,
    #[serde(rename = "session.delete")]
    SessionDelete,
    #[serde(rename = "flow.query")]
    FlowQuery,
    #[serde(rename = "batch.start")]
    BatchStart,
    #[serde(rename = "batch.validate")]
    BatchValidate,
    #[serde(rename = "batch.status")]
    BatchStatus,
    #[serde(rename = "batch.interrupt")]
    BatchInterrupt,
    #[serde(rename = "batch.cancel")]
    BatchCancel,
    #[serde(rename = "batch.list")]
    BatchList,
    #[serde(rename = "batch.resume")]
    BatchResume,
    #[serde(rename = "worker.shutdown")]
    WorkerShutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationMethod {
    #[serde(rename = "worker.ready")]
    WorkerReady,
    #[serde(rename = "worker.log")]
    WorkerLog,
    #[serde(rename = "job.progress")]
    JobProgress,
    #[serde(rename = "job.state_changed")]
    JobStateChanged,
    #[serde(rename = "job.completed")]
    JobCompleted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request<P> {
    pub api_version: u32,
    #[serde(rename = "type")]
    pub kind: MessageType,
    pub id: RequestId,
    pub method: RequestMethod,
    pub params: P,
}

impl<P> Request<P> {
    pub fn new(id: RequestId, method: RequestMethod, params: P) -> Self {
        Self {
            api_version: WORKER_API_VERSION,
            kind: MessageType::Request,
            id,
            method,
            params,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuccessResponse<R> {
    pub api_version: u32,
    #[serde(rename = "type")]
    pub kind: MessageType,
    pub id: RequestId,
    pub result: R,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse<D> {
    pub api_version: u32,
    #[serde(rename = "type")]
    pub kind: MessageType,
    pub id: Option<RequestId>,
    pub error: WorkerError<D>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response<R, D> {
    Success(SuccessResponse<R>),
    Error(ErrorResponse<D>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkerErrorCode {
    InvalidRequest,
    ProtocolVersionMismatch,
    MethodNotFound,
    InvalidParams,
    JobBusy,
    JobNotFound,
    SessionNotFound,
    CapturePermissionDenied,
    CoreUnavailable,
    Cancelled,
    InternalError,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerError<D> {
    pub code: WorkerErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<D>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notification<P> {
    pub api_version: u32,
    #[serde(rename = "type")]
    pub kind: MessageType,
    pub method: NotificationMethod,
    pub params: P,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StatusResult {
        status: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InvalidParamData {
        path: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct JobProgress {
        job_id: String,
        stage: String,
        progress: f64,
    }

    fn assert_roundtrip<T>(raw: &str)
    where
        T: DeserializeOwned + Serialize,
    {
        let parsed: T = serde_json::from_str(raw).expect("golden JSON should deserialize");
        let actual = serde_json::to_value(parsed).expect("protocol should serialize");
        let expected: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn golden_worker_messages_roundtrip() {
        assert_roundtrip::<Request<EmptyParams>>(include_str!("fixtures/worker-request-valid.json"));
        assert_roundtrip::<SuccessResponse<StatusResult>>(include_str!("fixtures/worker-response-valid.json"));
        assert_roundtrip::<ErrorResponse<InvalidParamData>>(include_str!("fixtures/worker-error-valid.json"));
        assert_roundtrip::<Notification<JobProgress>>(include_str!("fixtures/worker-notification-valid.json"));
    }

    #[test]
    fn request_constructor_sets_version_and_type() {
        let request = Request::new(RequestId::Integer(7), RequestMethod::Hello, EmptyParams::default());
        assert_eq!(request.api_version, WORKER_API_VERSION);
        assert_eq!(request.kind, MessageType::Request);
    }

    #[test]
    fn batch_interrupt_method_uses_worker_wire_name() {
        let value = serde_json::to_value(RequestMethod::BatchInterrupt).unwrap();
        assert_eq!(value, serde_json::json!("batch.interrupt"));
        let generic = serde_json::to_value(RequestMethod::JobInterrupt).unwrap();
        assert_eq!(generic, serde_json::json!("job.interrupt"));
        let validate = serde_json::to_value(RequestMethod::BatchValidate).unwrap();
        assert_eq!(validate, serde_json::json!("batch.validate"));
    }

    #[test]
    fn unknown_request_fields_and_methods_are_rejected() {
        let extra = r#"{
          "api_version":1,"type":"request","id":"x","method":"hello",
          "params":{},"unexpected":true
        }"#;
        assert!(serde_json::from_str::<Request<EmptyParams>>(extra).is_err());

        let unknown = r#"{
          "api_version":1,"type":"request","id":"x","method":"unknown",
          "params":{}
        }"#;
        assert!(serde_json::from_str::<Request<EmptyParams>>(unknown).is_err());
    }
}
