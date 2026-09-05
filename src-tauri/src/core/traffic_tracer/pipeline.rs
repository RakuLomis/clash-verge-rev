use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::schedule::PipelineSchedule;

pub const PIPELINE_SCHEMA_VERSION: u32 = 7;
const PIPELINE_MIN_SCHEMA_VERSION: u32 = 1;
pub const PIPELINE_MANIFEST_NAME: &str = "pipeline-manifest.json";
pub const PIPELINE_AGGREGATE_NAME: &str = "pipeline-aggregate.json";
pub const PIPELINE_MAX_REPETITIONS: u16 = 20;
pub const PIPELINE_FINGERPRINT_RUNTIME_BYTES_V1: &str = "runtime_bytes_v1";
pub const PIPELINE_FINGERPRINT_SEMANTIC_V2: &str = "runtime_semantic_v2";

fn default_repetitions() -> u16 {
    1
}

fn default_fingerprint_kind() -> String {
    PIPELINE_FINGERPRINT_RUNTIME_BYTES_V1.into()
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineState {
    Created,
    Validating,
    Running,
    Interrupted,
    Completed,
    CompletedWithDegraded,
    CompletedWithErrors,
    Failed,
    Cancelled,
    Restoring,
    RestoreFailed,
}

impl PipelineState {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::CompletedWithDegraded
                | Self::CompletedWithErrors
                | Self::Failed
                | Self::Cancelled
                | Self::RestoreFailed
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStage {
    Queued,
    Materializing,
    ActivatingProfile,
    WaitingController,
    SelectingProxy,
    DrainingConnections,
    Preflight,
    StartingBatch,
    RunningBatch,
    ReconcilingBatch,
    FinalizingBatch,
    VerifyingProtocol,
    Checkpoint,
    CaptureWave,
    AnalysisWave,
    Restoring,
    Finished,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunState {
    Pending,
    Running,
    Captured,
    Analyzing,
    RetryPending,
    Completed,
    Degraded,
    Failed,
    Interrupted,
    Skipped,
    Cancelled,
}

impl PipelineRunState {
    pub fn terminal(self) -> bool {
        !matches!(
            self,
            Self::Pending | Self::Running | Self::Captured | Self::Analyzing | Self::RetryPending
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineQualityPlane {
    pub state: String,
    pub passed: usize,
    pub degraded: usize,
    pub failed: usize,
    pub indeterminate: usize,
    pub not_applicable: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineApplicationIssue {
    pub session_id: String,
    pub target_url: String,
    pub final_url: Option<String>,
    #[serde(default)]
    pub final_status: Option<u16>,
    pub state: String,
    pub reason: Option<String>,
    pub primary_content_millis: Option<u64>,
    pub desired_primary_seconds: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineRunQuality {
    pub sessions_total: usize,
    pub capture_integrity: PipelineQualityPlane,
    pub correlation: PipelineQualityPlane,
    pub application: PipelineQualityPlane,
    pub application_issues: Vec<PipelineApplicationIssue>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineAggregateQuality {
    pub passed: usize,
    pub degraded: usize,
    pub failed: usize,
    pub indeterminate: usize,
    pub not_applicable: usize,
}

impl PipelineAggregateQuality {
    fn add(&mut self, plane: &PipelineQualityPlane) {
        self.passed += plane.passed;
        self.degraded += plane.degraded;
        self.failed += plane.failed;
        self.indeterminate += plane.indeterminate;
        self.not_applicable += plane.not_applicable;
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineCandidateAggregate {
    pub candidate_ordinal: u16,
    pub profile_uid: String,
    pub selection_group: String,
    pub requested_node: String,
    pub repetitions_planned: u16,
    pub repetitions_terminal: usize,
    pub cells_planned: usize,
    pub cells_terminal: usize,
    pub completed: usize,
    pub degraded: usize,
    pub failed: usize,
    pub interrupted: usize,
    pub cancelled: usize,
    pub sessions_total: usize,
    pub capture_integrity: PipelineAggregateQuality,
    pub correlation: PipelineAggregateQuality,
    pub application: PipelineAggregateQuality,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineAggregate {
    pub schema_version: u32,
    pub pipeline_id: String,
    pub updated_at: DateTime<Utc>,
    pub repetitions_per_candidate: u16,
    pub targets_planned: usize,
    pub planned_cells: usize,
    pub terminal_cells: usize,
    pub planned_runs: usize,
    pub terminal_runs: usize,
    pub candidates: Vec<PipelineCandidateAggregate>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineProxySnapshot {
    pub profile_uid: String,
    pub profile_fingerprint: String,
    pub selection_group: String,
    pub selected_node: String,
    pub resolved_chain: Vec<String>,
    pub resolved_leaf: String,
    pub protocol: String,
    pub captured_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineConnectionDrain {
    pub state: String,
    pub initial_connections: Option<usize>,
    pub final_connections: Option<usize>,
    pub polls: usize,
    pub quiet_millis: u64,
    pub error: Option<String>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineRunVerification {
    pub node_state: String,
    pub protocol_state: String,
    pub observed_protocols: Vec<String>,
    pub observed_selected_nodes: Vec<String>,
    pub observed_leaf_nodes: Vec<String>,
    pub details: Vec<String>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineProfileActivationStep {
    ActivationRequested,
    ProfileCommitted,
    ControllerVerified,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineProfileActivation {
    pub source_profile_uid: Option<String>,
    pub target_profile_uid: String,
    pub requested_at: DateTime<Utc>,
    pub profile_already_active: bool,
    pub resumed_from_committed_state: bool,
    pub profile_committed_at: Option<DateTime<Utc>>,
    pub controller_verified_at: Option<DateTime<Utc>>,
    pub last_completed_step: PipelineProfileActivationStep,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineRunEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_activation: Option<PipelineProfileActivation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_snapshot: Option<PipelineProxySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drain: Option<PipelineConnectionDrain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_batch_snapshot: Option<PipelineProxySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_snapshot: Option<PipelineProxySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<PipelineRunVerification>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineTarget {
    pub index: usize,
    pub url: String,
    pub domain: String,
    pub duration_seconds: u64,
    pub network: String,
    pub run_label: String,
    pub wait_load_timeout: u64,
    pub page_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineCandidate {
    pub profile_uid: String,
    pub profile_fingerprint: String,
    #[serde(default = "default_fingerprint_kind")]
    pub profile_fingerprint_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<DateTime<Utc>>,
    pub selection_group: String,
    pub requested_node: String,
}

impl PipelineCandidate {
    fn validate(&self) -> Result<()> {
        for (label, value) in [
            ("profile_uid", &self.profile_uid),
            ("selection_group", &self.selection_group),
            ("requested_node", &self.requested_node),
        ] {
            if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                bail!("pipeline candidate {label} is invalid");
            }
        }
        if self.profile_fingerprint.len() != 64
            || !self.profile_fingerprint.chars().all(|value| value.is_ascii_hexdigit())
        {
            bail!("pipeline candidate profile_fingerprint must be SHA-256");
        }
        if !matches!(
            self.profile_fingerprint_kind.as_str(),
            PIPELINE_FINGERPRINT_RUNTIME_BYTES_V1 | PIPELINE_FINGERPRINT_SEMANTIC_V2
        ) {
            bail!("pipeline candidate profile_fingerprint_kind is unsupported");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelinePolicy {
    pub continue_on_run_failure: bool,
    pub restore_original_state: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineSelection {
    pub group: String,
    pub node: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineRestoreCheck {
    pub component: String,
    pub target: String,
    pub requested: String,
    pub observed: Option<String>,
    pub state: String,
    pub code: Option<String>,
    pub message: Option<String>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreState {
    Pending,
    NotRequired,
    Restoring,
    Restored,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineRestore {
    pub profile_uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_state: Option<PipelineState>,
    pub selections: Vec<PipelineSelection>,
    #[serde(default)]
    pub checks: Vec<PipelineRestoreCheck>,
    pub state: RestoreState,
    pub error: Option<PipelineError>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineRun {
    pub ordinal: usize,
    #[serde(default = "default_repetitions")]
    pub candidate_ordinal: u16,
    #[serde(default = "default_repetitions")]
    pub repetition_index: u16,
    #[serde(default = "default_repetitions")]
    pub repetition_total: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_index: Option<usize>,
    pub run_id: String,
    pub profile_uid: String,
    pub profile_fingerprint: String,
    #[serde(default = "default_fingerprint_kind")]
    pub profile_fingerprint_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_profile_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_bound_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub profile_snapshot_changed: bool,
    pub selection_group: String,
    pub requested_node: String,
    pub state: PipelineRunState,
    pub stage: PipelineStage,
    pub resolved_chain: Vec<String>,
    pub resolved_leaf: Option<String>,
    pub expected_protocol: String,
    pub observed_protocol: String,
    pub batch_id: Option<String>,
    pub output_path: PathBuf,
    #[serde(default)]
    pub session_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_job_id: Option<String>,
    #[serde(default)]
    pub application_retry_attempt: u8,
    #[serde(default)]
    pub prior_session_ids: Vec<String>,
    pub error: Option<PipelineError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<PipelineRunQuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<PipelineRunEvidence>,
    pub resume_attempt: u32,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineConfigSnapshot {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineManifest {
    pub schema_version: u32,
    pub pipeline_id: String,
    pub state: PipelineState,
    pub stage: PipelineStage,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub output_root: PathBuf,
    pub config: PipelineConfigSnapshot,
    pub targets: Vec<PipelineTarget>,
    pub execution: serde_json::Value,
    pub policy: PipelinePolicy,
    #[serde(default)]
    pub schedule: PipelineSchedule,
    #[serde(default = "default_repetitions")]
    pub repetitions_per_candidate: u16,
    pub current_run_index: Option<usize>,
    pub runs: Vec<PipelineRun>,
    pub restore: PipelineRestore,
}

impl PipelineManifest {
    pub fn create(
        pipeline_id: String,
        output_root: PathBuf,
        config: PipelineConfigSnapshot,
        targets: Vec<PipelineTarget>,
        execution: serde_json::Value,
        candidates: Vec<(PipelineCandidate, Vec<String>)>,
        repetitions_per_candidate: u16,
        policy: PipelinePolicy,
        restore: PipelineRestore,
    ) -> Result<Self> {
        if targets.is_empty() || candidates.is_empty() {
            bail!("pipeline requires at least one target and candidate");
        }
        if !(1..=PIPELINE_MAX_REPETITIONS).contains(&repetitions_per_candidate) {
            bail!("pipeline repetitions_per_candidate must be between 1 and {PIPELINE_MAX_REPETITIONS}");
        }
        if !output_root.is_absolute() || !config.path.is_absolute() {
            bail!("pipeline paths must be absolute");
        }
        if config.sha256.len() != 64 || !config.sha256.chars().all(|value| value.is_ascii_hexdigit()) {
            bail!("pipeline config sha256 must be SHA-256");
        }
        let mut identities = HashSet::new();
        let mut runs = Vec::with_capacity(candidates.len() * usize::from(repetitions_per_candidate));
        for (candidate_index, (candidate, run_ids)) in candidates.into_iter().enumerate() {
            candidate.validate()?;
            let identity = (
                candidate.profile_uid.clone(),
                candidate.selection_group.clone(),
                candidate.requested_node.clone(),
            );
            if !identities.insert(identity) {
                bail!("pipeline candidates must be unique by profile, selector and node");
            }
            if run_ids.len() != usize::from(repetitions_per_candidate) {
                bail!("pipeline candidate run-id count does not match repetitions_per_candidate");
            }
            for (repetition_index, run_id) in run_ids.into_iter().enumerate() {
                let ordinal = runs.len() + 1;
                runs.push(PipelineRun {
                    ordinal,
                    candidate_ordinal: u16::try_from(candidate_index + 1)
                        .context("pipeline candidate ordinal overflow")?,
                    repetition_index: u16::try_from(repetition_index + 1)
                        .context("pipeline repetition index overflow")?,
                    repetition_total: repetitions_per_candidate,
                    run_id: run_id.clone(),
                    profile_uid: candidate.profile_uid.clone(),
                    target_index: None,
                    profile_fingerprint: candidate.profile_fingerprint.clone(),
                    profile_fingerprint_kind: candidate.profile_fingerprint_kind.clone(),
                    queued_profile_fingerprint: Some(candidate.profile_fingerprint.clone()),
                    profile_bound_at: None,
                    profile_snapshot_changed: false,
                    selection_group: candidate.selection_group.clone(),
                    requested_node: candidate.requested_node.clone(),
                    state: PipelineRunState::Pending,
                    stage: PipelineStage::Queued,
                    resolved_chain: Vec::new(),
                    resolved_leaf: None,
                    expected_protocol: String::new(),
                    observed_protocol: String::new(),
                    batch_id: None,
                    output_path: output_root.join("runs").join(format!(
                        "{:03}_candidate-{:02}_repeat-{:02}_{}",
                        ordinal,
                        candidate_index + 1,
                        repetition_index + 1,
                        &run_id[..12.min(run_id.len())]
                    )),
                    session_ids: Vec::new(),
                    analysis_job_id: None,
                    application_retry_attempt: 0,
                    prior_session_ids: Vec::new(),
                    error: None,
                    quality: None,
                    evidence: None,
                    resume_attempt: 0,
                    started_at: None,
                    completed_at: None,
                });
            }
        }
        let now = Utc::now();
        Ok(Self {
            schema_version: PIPELINE_SCHEMA_VERSION,
            pipeline_id,
            state: PipelineState::Created,
            stage: PipelineStage::Queued,
            created_at: now,
            updated_at: now,
            output_root,
            config,
            targets,
            execution,
            policy,
            schedule: PipelineSchedule::default(),
            repetitions_per_candidate,
            current_run_index: None,
            runs,
            restore,
        })
    }

    pub fn create_matrix(
        pipeline_id: String,
        output_root: PathBuf,
        config: PipelineConfigSnapshot,
        targets: Vec<PipelineTarget>,
        execution: serde_json::Value,
        candidates: Vec<PipelineCandidate>,
        run_ids: Vec<String>,
        repetitions_per_candidate: u16,
        policy: PipelinePolicy,
        restore: PipelineRestore,
        schedule: PipelineSchedule,
    ) -> Result<Self> {
        if targets.is_empty() || candidates.is_empty() {
            bail!("pipeline requires at least one target and candidate");
        }
        if !(1..=PIPELINE_MAX_REPETITIONS).contains(&repetitions_per_candidate) {
            bail!("pipeline repetitions_per_candidate must be between 1 and {PIPELINE_MAX_REPETITIONS}");
        }
        if !output_root.is_absolute() || !config.path.is_absolute() {
            bail!("pipeline paths must be absolute");
        }
        if config.sha256.len() != 64 || !config.sha256.chars().all(|value| value.is_ascii_hexdigit()) {
            bail!("pipeline config sha256 must be SHA-256");
        }
        schedule.validate(repetitions_per_candidate, candidates.len())?;
        let expected_runs = usize::from(repetitions_per_candidate)
            .checked_mul(targets.len())
            .and_then(|value| value.checked_mul(candidates.len()))
            .context("pipeline matrix size overflow")?;
        if run_ids.len() != expected_runs {
            bail!("pipeline matrix run-id count does not match repetitions, targets and candidates");
        }

        let mut identities = HashSet::new();
        for candidate in &candidates {
            candidate.validate()?;
            let identity = (
                candidate.profile_uid.clone(),
                candidate.selection_group.clone(),
                candidate.requested_node.clone(),
            );
            if !identities.insert(identity) {
                bail!("pipeline candidates must be unique by profile, selector and node");
            }
        }

        let mut ids = run_ids.into_iter();
        let mut runs = Vec::with_capacity(expected_runs);
        for (repetition_offset, candidate_order) in schedule.repetition_candidate_orders.iter().enumerate() {
            for target in &targets {
                for candidate_ordinal in candidate_order {
                    let candidate_index = usize::from(*candidate_ordinal - 1);
                    let candidate = &candidates[candidate_index];
                    let run_id = ids.next().context("pipeline matrix run-id is missing")?;
                    let ordinal = runs.len() + 1;
                    let repetition_index =
                        u16::try_from(repetition_offset + 1).context("pipeline repetition index overflow")?;
                    runs.push(PipelineRun {
                        ordinal,
                        candidate_ordinal: *candidate_ordinal,
                        repetition_index,
                        repetition_total: repetitions_per_candidate,
                        target_index: Some(target.index),
                        run_id: run_id.clone(),
                        profile_uid: candidate.profile_uid.clone(),
                        profile_fingerprint: candidate.profile_fingerprint.clone(),
                        profile_fingerprint_kind: candidate.profile_fingerprint_kind.clone(),
                        queued_profile_fingerprint: Some(candidate.profile_fingerprint.clone()),
                        profile_bound_at: None,
                        profile_snapshot_changed: false,
                        selection_group: candidate.selection_group.clone(),
                        requested_node: candidate.requested_node.clone(),
                        state: PipelineRunState::Pending,
                        stage: PipelineStage::Queued,
                        resolved_chain: Vec::new(),
                        resolved_leaf: None,
                        expected_protocol: String::new(),
                        observed_protocol: String::new(),
                        batch_id: None,
                        session_ids: Vec::new(),
                        analysis_job_id: None,
                        application_retry_attempt: 0,
                        prior_session_ids: Vec::new(),
                        output_path: output_root.join("runs").join(format!(
                            "{:04}_repeat-{:02}_target-{:03}_candidate-{:02}_{}",
                            ordinal,
                            repetition_index,
                            target.index,
                            candidate_ordinal,
                            &run_id[..12.min(run_id.len())]
                        )),
                        error: None,
                        quality: None,
                        evidence: None,
                        resume_attempt: 0,
                        started_at: None,
                        completed_at: None,
                    });
                }
            }
        }

        let now = Utc::now();
        Ok(Self {
            schema_version: PIPELINE_SCHEMA_VERSION,
            pipeline_id,
            state: PipelineState::Created,
            stage: PipelineStage::Queued,
            created_at: now,
            updated_at: now,
            output_root,
            config,
            targets,
            execution,
            policy,
            schedule,
            repetitions_per_candidate,
            current_run_index: None,
            runs,
            restore,
        })
    }
    pub fn aggregate(&self) -> PipelineAggregate {
        let mut candidates = Vec::new();
        for candidate_ordinal in 1..=self.runs.iter().map(|run| run.candidate_ordinal).max().unwrap_or(0) {
            let candidate_runs = self
                .runs
                .iter()
                .filter(|run| run.candidate_ordinal == candidate_ordinal)
                .collect::<Vec<_>>();
            let Some(first) = candidate_runs.first() else { continue };
            let repetitions_terminal = (1..=self.repetitions_per_candidate)
                .filter(|repetition| {
                    let cells = candidate_runs
                        .iter()
                        .filter(|run| run.repetition_index == *repetition)
                        .collect::<Vec<_>>();
                    !cells.is_empty() && cells.iter().all(|run| run.state.terminal())
                })
                .count();
            let mut aggregate = PipelineCandidateAggregate {
                candidate_ordinal,
                profile_uid: first.profile_uid.clone(),
                selection_group: first.selection_group.clone(),
                requested_node: first.requested_node.clone(),
                repetitions_planned: self.repetitions_per_candidate,
                repetitions_terminal,
                cells_planned: candidate_runs.len(),
                cells_terminal: candidate_runs.iter().filter(|run| run.state.terminal()).count(),
                completed: 0,
                degraded: 0,
                failed: 0,
                interrupted: 0,
                cancelled: 0,
                sessions_total: 0,
                capture_integrity: PipelineAggregateQuality::default(),
                correlation: PipelineAggregateQuality::default(),
                application: PipelineAggregateQuality::default(),
            };
            for run in candidate_runs {
                match run.state {
                    PipelineRunState::Completed => aggregate.completed += 1,
                    PipelineRunState::Degraded => aggregate.degraded += 1,
                    PipelineRunState::Failed | PipelineRunState::Skipped => aggregate.failed += 1,
                    PipelineRunState::Interrupted => aggregate.interrupted += 1,
                    PipelineRunState::Cancelled => aggregate.cancelled += 1,
                    PipelineRunState::Pending
                    | PipelineRunState::Running
                    | PipelineRunState::Captured
                    | PipelineRunState::Analyzing
                    | PipelineRunState::RetryPending => {}
                }
                if let Some(quality) = &run.quality {
                    aggregate.sessions_total += quality.sessions_total;
                    aggregate.capture_integrity.add(&quality.capture_integrity);
                    aggregate.correlation.add(&quality.correlation);
                    aggregate.application.add(&quality.application);
                }
            }
            candidates.push(aggregate);
        }
        PipelineAggregate {
            schema_version: 2,
            pipeline_id: self.pipeline_id.clone(),
            updated_at: self.updated_at,
            repetitions_per_candidate: self.repetitions_per_candidate,
            targets_planned: self.targets.len(),
            planned_cells: self.runs.len(),
            terminal_cells: self.runs.iter().filter(|run| run.state.terminal()).count(),
            planned_runs: self.runs.len(),
            terminal_runs: self.runs.iter().filter(|run| run.state.terminal()).count(),
            candidates,
        }
    }

    pub fn persist(&self) -> Result<PathBuf> {
        fs::create_dir_all(&self.output_root).context("create pipeline output directory")?;
        let path = self.output_root.join(PIPELINE_MANIFEST_NAME);
        let temporary = self.output_root.join(format!(".{PIPELINE_MANIFEST_NAME}.tmp"));
        let mut bytes = serde_json::to_vec_pretty(self).context("serialize pipeline manifest")?;
        bytes.push(b'\n');
        fs::write(&temporary, bytes).context("write pipeline manifest checkpoint")?;
        fs::rename(&temporary, &path).context("commit pipeline manifest checkpoint")?;

        let aggregate_path = self.output_root.join(PIPELINE_AGGREGATE_NAME);
        let aggregate_temporary = self.output_root.join(format!(".{PIPELINE_AGGREGATE_NAME}.tmp"));
        let mut aggregate_bytes =
            serde_json::to_vec_pretty(&self.aggregate()).context("serialize pipeline aggregate")?;
        aggregate_bytes.push(b'\n');
        fs::write(&aggregate_temporary, aggregate_bytes).context("write pipeline aggregate checkpoint")?;
        fs::rename(&aggregate_temporary, &aggregate_path).context("commit pipeline aggregate checkpoint")?;
        Ok(path)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let data = fs::read(path.as_ref()).context("read pipeline manifest")?;
        let mut manifest: Self = serde_json::from_slice(&data).context("decode pipeline manifest")?;
        if !(PIPELINE_MIN_SCHEMA_VERSION..=PIPELINE_SCHEMA_VERSION).contains(&manifest.schema_version) {
            bail!("unsupported pipeline manifest schema version");
        }
        let loaded_schema_version = manifest.schema_version;
        if loaded_schema_version < 4 {
            manifest.repetitions_per_candidate = 1;
            for run in &mut manifest.runs {
                run.candidate_ordinal =
                    u16::try_from(run.ordinal).context("legacy pipeline candidate ordinal overflow")?;
                run.repetition_index = 1;
                run.repetition_total = 1;
            }
        }
        if loaded_schema_version < 6 {
            for run in &mut manifest.runs {
                run.profile_fingerprint_kind = PIPELINE_FINGERPRINT_RUNTIME_BYTES_V1.into();
                run.queued_profile_fingerprint
                    .get_or_insert_with(|| run.profile_fingerprint.clone());
                run.profile_bound_at = None;
                run.profile_snapshot_changed = false;
            }
            manifest.restore.profile_fingerprint = None;
        }
        if loaded_schema_version < 7 {
            manifest.schedule = PipelineSchedule::default();
        }
        let candidate_count = manifest.runs.iter().map(|run| run.candidate_ordinal).max().unwrap_or(0);
        manifest
            .schedule
            .validate(manifest.repetitions_per_candidate, usize::from(candidate_count))?;
        if manifest.schedule.mode == super::schedule::PipelineScheduleMode::RepetitionTargetCandidate {
            let expected = usize::from(manifest.repetitions_per_candidate)
                .checked_mul(manifest.targets.len())
                .and_then(|value| value.checked_mul(usize::from(candidate_count)))
                .context("pipeline matrix size overflow")?;
            if manifest.runs.len() != expected {
                bail!("pipeline matrix run count does not match its frozen schedule");
            }
            let mut position = 0;
            for (repetition_offset, order) in manifest.schedule.repetition_candidate_orders.iter().enumerate() {
                for target in &manifest.targets {
                    for candidate_ordinal in order {
                        let run = &manifest.runs[position];
                        let repetition_index =
                            u16::try_from(repetition_offset + 1).context("pipeline repetition index overflow")?;
                        if run.ordinal != position + 1
                            || run.repetition_index != repetition_index
                            || run.target_index != Some(target.index)
                            || run.candidate_ordinal != *candidate_ordinal
                        {
                            bail!("pipeline matrix run order does not match its frozen schedule");
                        }
                        position += 1;
                    }
                }
            }
        }
        manifest.schema_version = PIPELINE_SCHEMA_VERSION;
        Ok(manifest)
    }

    pub fn recover_interrupted_supervisor(&mut self) -> Result<bool> {
        if !matches!(
            self.state,
            PipelineState::Running | PipelineState::Validating | PipelineState::Restoring
        ) {
            return Ok(false);
        }
        if let Some(index) = self.current_run_index {
            if self.schedule.mode == super::schedule::PipelineScheduleMode::RepetitionTargetCandidate
                && self.runs[index].state == PipelineRunState::Analyzing
            {
                self.runs[index].state = PipelineRunState::Captured;
                self.runs[index].stage = PipelineStage::Checkpoint;
                self.runs[index].error = Some(PipelineError {
                    code: "PIPELINE_ANALYSIS_SUPERVISOR_RESTARTED".into(),
                    message: "The application stopped during deferred analysis; resume will analyze the existing raw Session without recapturing it.".into(),
                });
                self.current_run_index = None;
            } else {
                self.finish_run(
                    PipelineRunState::Interrupted,
                    Some(PipelineError {
                        code: "PIPELINE_SUPERVISOR_RESTARTED".into(),
                        message: "The application stopped while this pipeline capture was active; resume can continue its Batch checkpoint.".into(),
                    }),
                )?;
            }
        }
        self.state = PipelineState::Interrupted;
        self.stage = PipelineStage::Finished;
        self.updated_at = Utc::now();
        Ok(true)
    }

    pub fn begin_next_run(&mut self) -> Result<Option<usize>> {
        if !matches!(
            self.state,
            PipelineState::Created | PipelineState::Interrupted | PipelineState::Running
        ) {
            bail!("pipeline cannot begin a run from its current state");
        }
        if self.current_run_index.is_some() {
            bail!("pipeline already has an active run");
        }
        let Some(index) = self
            .runs
            .iter()
            .position(|run| matches!(run.state, PipelineRunState::Pending | PipelineRunState::Interrupted))
        else {
            self.state = if self
                .runs
                .iter()
                .any(|run| matches!(run.state, PipelineRunState::Failed | PipelineRunState::Skipped))
            {
                PipelineState::CompletedWithErrors
            } else if self.runs.iter().any(|run| run.state == PipelineRunState::Degraded) {
                PipelineState::CompletedWithDegraded
            } else {
                PipelineState::Completed
            };
            self.stage = PipelineStage::Finished;
            self.updated_at = Utc::now();
            return Ok(None);
        };
        let run = &mut self.runs[index];
        let resuming_interrupted = run.state == PipelineRunState::Interrupted;
        if resuming_interrupted {
            run.resume_attempt += 1;
        }
        run.state = PipelineRunState::Running;
        run.stage = PipelineStage::ActivatingProfile;
        run.error = None;
        run.quality = None;
        if !resuming_interrupted {
            run.evidence = None;
        }
        run.started_at.get_or_insert_with(Utc::now);
        self.state = PipelineState::Running;
        self.stage = PipelineStage::ActivatingProfile;
        self.current_run_index = Some(index);
        self.updated_at = Utc::now();
        Ok(Some(index))
    }

    pub fn begin_next_capture(&mut self, repetition_index: u16) -> Result<Option<usize>> {
        if self.current_run_index.is_some() {
            bail!("pipeline already has an active run");
        }
        let Some(index) = self.runs.iter().position(|run| {
            run.repetition_index == repetition_index
                && matches!(
                    run.state,
                    PipelineRunState::Pending | PipelineRunState::Interrupted | PipelineRunState::RetryPending
                )
        }) else {
            return Ok(None);
        };
        let run = &mut self.runs[index];
        let resuming = run.state == PipelineRunState::Interrupted;
        if resuming {
            run.resume_attempt += 1;
        }
        run.state = PipelineRunState::Running;
        run.stage = PipelineStage::ActivatingProfile;
        run.error = None;
        run.quality = None;
        if !resuming {
            run.evidence = None;
            run.session_ids.clear();
            run.analysis_job_id = None;
        }
        run.started_at.get_or_insert_with(Utc::now);
        self.state = PipelineState::Running;
        self.stage = PipelineStage::CaptureWave;
        self.current_run_index = Some(index);
        self.updated_at = Utc::now();
        Ok(Some(index))
    }

    pub fn finish_capture(&mut self, session_ids: Vec<String>) -> Result<()> {
        let Some(index) = self.current_run_index.take() else {
            bail!("pipeline has no active run");
        };
        if self.runs[index].state != PipelineRunState::Running {
            bail!("pipeline capture is not active");
        }
        if session_ids.is_empty() {
            bail!("pipeline capture did not produce a Session");
        }
        self.runs[index].state = PipelineRunState::Captured;
        self.runs[index].stage = PipelineStage::Checkpoint;
        self.runs[index].session_ids = session_ids;
        self.runs[index].error = None;
        self.stage = PipelineStage::CaptureWave;
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn begin_next_analysis(&mut self, repetition_index: u16) -> Result<Option<usize>> {
        if self.current_run_index.is_some() {
            bail!("pipeline already has an active run");
        }
        if self.runs.iter().any(|run| {
            run.repetition_index == repetition_index
                && matches!(
                    run.state,
                    PipelineRunState::Pending | PipelineRunState::Running | PipelineRunState::Interrupted
                )
        }) {
            bail!("pipeline analysis barrier requires the repetition capture wave to finish");
        }
        let Some(index) = self
            .runs
            .iter()
            .position(|run| run.repetition_index == repetition_index && run.state == PipelineRunState::Captured)
        else {
            return Ok(None);
        };
        self.runs[index].state = PipelineRunState::Analyzing;
        self.runs[index].stage = PipelineStage::AnalysisWave;
        self.runs[index].error = None;
        self.state = PipelineState::Running;
        self.stage = PipelineStage::AnalysisWave;
        self.current_run_index = Some(index);
        self.updated_at = Utc::now();
        Ok(Some(index))
    }

    pub fn finish_analysis(&mut self, state: PipelineRunState, error: Option<PipelineError>) -> Result<()> {
        if !state.terminal() {
            bail!("pipeline analysis finish state must be terminal");
        }
        let Some(index) = self.current_run_index.take() else {
            bail!("pipeline has no active run");
        };
        if self.runs[index].state != PipelineRunState::Analyzing {
            bail!("pipeline analysis is not active");
        }
        self.runs[index].state = state;
        self.runs[index].stage = PipelineStage::Finished;
        self.runs[index].error = error;
        self.runs[index].completed_at = Some(Utc::now());
        self.stage = PipelineStage::Checkpoint;
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn schedule_application_retry(&mut self, max_retries: u8) -> Result<()> {
        let Some(index) = self.current_run_index.take() else {
            bail!("pipeline has no active analysis");
        };
        let run = &mut self.runs[index];
        if run.state != PipelineRunState::Analyzing {
            bail!("pipeline application retry requires active analysis");
        }
        if max_retries == 0 || run.application_retry_attempt >= max_retries {
            bail!("pipeline application retry budget is exhausted");
        }
        run.prior_session_ids.append(&mut run.session_ids);
        run.application_retry_attempt += 1;
        run.state = PipelineRunState::RetryPending;
        run.stage = PipelineStage::CaptureWave;
        run.batch_id = None;
        run.analysis_job_id = None;
        run.error = None;
        run.completed_at = None;
        self.stage = PipelineStage::CaptureWave;
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn finalize_matrix(&mut self) -> Result<()> {
        if self.current_run_index.is_some() || self.runs.iter().any(|run| !run.state.terminal()) {
            bail!("pipeline matrix cannot finish while cells remain nonterminal");
        }
        self.state = if self
            .runs
            .iter()
            .any(|run| matches!(run.state, PipelineRunState::Failed | PipelineRunState::Skipped))
        {
            PipelineState::CompletedWithErrors
        } else if self.runs.iter().any(|run| run.state == PipelineRunState::Degraded) {
            PipelineState::CompletedWithDegraded
        } else if self.runs.iter().any(|run| run.state == PipelineRunState::Cancelled) {
            PipelineState::Cancelled
        } else {
            PipelineState::Completed
        };
        self.stage = PipelineStage::Finished;
        self.updated_at = Utc::now();
        Ok(())
    }
    pub fn checkpoint_run(&mut self, stage: PipelineStage) -> Result<()> {
        let Some(index) = self.current_run_index else {
            bail!("pipeline has no active run");
        };
        if self.runs[index].state != PipelineRunState::Running {
            bail!("pipeline run is not active");
        }
        self.runs[index].stage = stage;
        self.stage = stage;
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn finish_run(&mut self, state: PipelineRunState, error: Option<PipelineError>) -> Result<()> {
        if !state.terminal() {
            bail!("pipeline run finish state must be terminal");
        }
        let Some(index) = self.current_run_index.take() else {
            bail!("pipeline has no active run");
        };
        self.runs[index].state = state;
        self.runs[index].stage = PipelineStage::Finished;
        self.runs[index].error = error;
        self.runs[index].completed_at = Some(Utc::now());
        self.stage = PipelineStage::Checkpoint;
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn bind_candidate_profile(
        &mut self,
        candidate_ordinal: u16,
        fingerprint: String,
        bound_at: DateTime<Utc>,
    ) -> Result<bool> {
        if fingerprint.len() != 64 || !fingerprint.chars().all(|value| value.is_ascii_hexdigit()) {
            bail!("pipeline bound profile fingerprint must be SHA-256");
        }
        let mut found = false;
        let mut changed = false;
        for run in self
            .runs
            .iter_mut()
            .filter(|run| run.candidate_ordinal == candidate_ordinal)
        {
            found = true;
            let run_changed = run.profile_fingerprint != fingerprint
                || run.profile_fingerprint_kind != PIPELINE_FINGERPRINT_SEMANTIC_V2;
            changed |= run_changed;
            run.profile_snapshot_changed = run_changed;
            run.profile_fingerprint = fingerprint.clone();
            run.profile_fingerprint_kind = PIPELINE_FINGERPRINT_SEMANTIC_V2.into();
            run.profile_bound_at = Some(bound_at);
        }
        if !found {
            bail!("pipeline candidate ordinal is absent");
        }
        self.updated_at = Utc::now();
        Ok(changed)
    }

    pub fn fail_candidate_materialization(&mut self, candidate_ordinal: u16, error: PipelineError) -> Result<()> {
        let now = Utc::now();
        let mut first = true;
        let mut found = false;
        for run in self.runs.iter_mut().filter(|run| {
            run.candidate_ordinal == candidate_ordinal
                && matches!(run.state, PipelineRunState::Pending | PipelineRunState::Interrupted)
        }) {
            found = true;
            run.state = if first {
                PipelineRunState::Failed
            } else {
                PipelineRunState::Skipped
            };
            run.stage = PipelineStage::Finished;
            run.error = Some(if first {
                error.clone()
            } else {
                PipelineError {
                    code: "CANDIDATE_BLOCKED".into(),
                    message: format!(
                        "Skipped because candidate materialization failed: {}: {}",
                        error.code, error.message
                    ),
                }
            });
            run.started_at.get_or_insert(now);
            run.completed_at = Some(now);
            first = false;
        }
        if !found {
            bail!("pipeline candidate has no pending runs to fail");
        }
        self.updated_at = now;
        Ok(())
    }

    pub fn skip_remaining_candidate_runs(&mut self, candidate_ordinal: u16, cause: &PipelineError) -> usize {
        let now = Utc::now();
        let mut skipped = 0;
        for run in self
            .runs
            .iter_mut()
            .filter(|run| run.candidate_ordinal == candidate_ordinal && run.state == PipelineRunState::Pending)
        {
            run.state = PipelineRunState::Skipped;
            run.stage = PipelineStage::Finished;
            run.error = Some(PipelineError {
                code: "CANDIDATE_BLOCKED".into(),
                message: format!(
                    "Skipped after non-retryable candidate error: {}: {}",
                    cause.code, cause.message
                ),
            });
            run.started_at = Some(now);
            run.completed_at = Some(now);
            skipped += 1;
        }
        if skipped > 0 {
            self.updated_at = now;
        }
        skipped
    }

    pub fn skip_remaining_runs(&mut self, cause: &PipelineError) -> usize {
        let now = Utc::now();
        let mut skipped = 0;
        for run in self.runs.iter_mut().filter(|run| {
            matches!(
                run.state,
                PipelineRunState::Pending
                    | PipelineRunState::Interrupted
                    | PipelineRunState::RetryPending
                    | PipelineRunState::Captured
            )
        }) {
            run.state = PipelineRunState::Skipped;
            run.stage = PipelineStage::Finished;
            run.error = Some(PipelineError {
                code: "PIPELINE_SYSTEMIC_FAILURE".into(),
                message: format!(
                    "Skipped because the shared capture/analysis infrastructure failed: {}: {}",
                    cause.code, cause.message
                ),
            });
            run.started_at.get_or_insert(now);
            run.completed_at = Some(now);
            skipped += 1;
        }
        if skipped > 0 {
            self.updated_at = now;
        }
        skipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(node: &str) -> PipelineCandidate {
        PipelineCandidate {
            profile_uid: "profile-one".into(),
            profile_fingerprint: "a".repeat(64),
            profile_fingerprint_kind: PIPELINE_FINGERPRINT_SEMANTIC_V2.into(),
            recorded_at: Some(Utc::now()),
            selection_group: "GLOBAL".into(),
            requested_node: node.into(),
        }
    }

    fn target() -> PipelineTarget {
        PipelineTarget {
            index: 0,
            url: "https://example.com/".into(),
            domain: "example.com".into(),
            duration_seconds: 8,
            network: "all".into(),
            run_label: "example".into(),
            wait_load_timeout: 30,
            page_type: "example-page".into(),
            playback: None,
        }
    }

    #[test]
    fn creates_ordered_runs_and_rejects_duplicate_identity() {
        let build = |candidates| {
            PipelineManifest::create(
                "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
                PathBuf::from("/tmp/pipeline"),
                PipelineConfigSnapshot {
                    path: PathBuf::from("/tmp/sites.yaml"),
                    sha256: "b".repeat(64),
                },
                vec![target()],
                serde_json::json!({"tun_interface":"Meta"}),
                candidates,
                1,
                PipelinePolicy {
                    continue_on_run_failure: true,
                    restore_original_state: true,
                },
                PipelineRestore {
                    profile_uid: Some("profile-one".into()),
                    profile_fingerprint: None,
                    terminal_state: None,
                    selections: vec![],
                    checks: vec![],
                    state: RestoreState::Pending,
                    error: None,
                },
            )
        };
        let manifest = build(vec![
            (candidate("one"), vec!["e107516f-335d-42f5-b9f4-f71c081c41e7".into()]),
            (candidate("two"), vec!["e38c26b7-789c-4aa0-b1bb-e3d5916390af".into()]),
        ])
        .unwrap();
        assert_eq!(
            manifest.runs.iter().map(|run| run.ordinal).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(
            build(vec![
                (candidate("same"), vec!["e107516f-335d-42f5-b9f4-f71c081c41e7".into()]),
                (candidate("same"), vec!["e38c26b7-789c-4aa0-b1bb-e3d5916390af".into()])
            ])
            .is_err()
        );
    }

    #[test]
    fn expands_candidates_in_candidate_major_repetition_order() {
        let manifest = PipelineManifest::create(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            PathBuf::from("/tmp/pipeline"),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({"tun_interface":"Meta"}),
            vec![
                (candidate("one"), vec!["one-r1".into(), "one-r2".into()]),
                (candidate("two"), vec!["two-r1".into(), "two-r2".into()]),
            ],
            2,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();

        assert_eq!(manifest.repetitions_per_candidate, 2);
        assert_eq!(
            manifest
                .runs
                .iter()
                .map(|run| (run.candidate_ordinal, run.repetition_index, run.requested_node.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, 1, "one"), (1, 2, "one"), (2, 1, "two"), (2, 2, "two")]
        );
        assert!(
            manifest.runs[0]
                .output_path
                .to_string_lossy()
                .contains("candidate-01_repeat-01")
        );
        assert!(
            manifest.runs[3]
                .output_path
                .to_string_lossy()
                .contains("candidate-02_repeat-02")
        );
        let aggregate = manifest.aggregate();
        assert_eq!(aggregate.planned_runs, 4);
        assert_eq!(aggregate.candidates.len(), 2);
    }

    #[test]
    fn checkpoints_runs_strictly_in_order_and_reports_degraded_completion() {
        let mut manifest = PipelineManifest::create(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            PathBuf::from("/tmp/pipeline"),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({"tun_interface":"Meta"}),
            vec![
                (candidate("one"), vec!["e107516f-335d-42f5-b9f4-f71c081c41e7".into()]),
                (candidate("two"), vec!["e38c26b7-789c-4aa0-b1bb-e3d5916390af".into()]),
            ],
            1,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();
        assert_eq!(manifest.begin_next_run().unwrap(), Some(0));
        manifest.checkpoint_run(PipelineStage::RunningBatch).unwrap();
        manifest
            .finish_run(
                PipelineRunState::Failed,
                Some(PipelineError {
                    code: "NODE_FAILED".into(),
                    message: "node failed".into(),
                }),
            )
            .unwrap();
        assert_eq!(manifest.begin_next_run().unwrap(), Some(1));
        manifest.finish_run(PipelineRunState::Completed, None).unwrap();
        assert_eq!(manifest.begin_next_run().unwrap(), None);
        assert_eq!(manifest.state, PipelineState::CompletedWithErrors);
    }

    #[test]
    fn reports_degraded_completion_separately_from_errors() {
        let mut manifest = PipelineManifest::create(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            PathBuf::from("/tmp/pipeline"),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({}),
            vec![(candidate("one"), vec!["one-r1".into()])],
            1,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();
        manifest.begin_next_run().unwrap();
        manifest.finish_run(PipelineRunState::Degraded, None).unwrap();
        assert_eq!(manifest.begin_next_run().unwrap(), None);
        assert_eq!(manifest.state, PipelineState::CompletedWithDegraded);
    }

    #[test]
    fn persists_and_loads_a_manifest_atomically() {
        let root = std::env::temp_dir().join(format!("traffictracer-pipeline-model-{}", std::process::id()));
        let mut manifest = PipelineManifest::create(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            root.clone(),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({"tun_interface":"Meta"}),
            vec![(candidate("one"), vec!["e107516f-335d-42f5-b9f4-f71c081c41e7".into()])],
            1,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();
        manifest.begin_next_run().unwrap();
        let path = manifest.persist().unwrap();
        assert_eq!(PipelineManifest::load(path).unwrap(), manifest);
        let aggregate: PipelineAggregate =
            serde_json::from_slice(&fs::read(root.join(PIPELINE_AGGREGATE_NAME)).unwrap()).unwrap();
        assert_eq!(aggregate.pipeline_id, manifest.pipeline_id);
        assert_eq!(aggregate.planned_runs, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn loads_schema_one_without_quality_and_migrates_in_memory() {
        let root = std::env::temp_dir().join(format!("traffictracer-pipeline-v1-model-{}", std::process::id()));
        let manifest = PipelineManifest::create(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            root.clone(),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({"tun_interface":"Meta"}),
            vec![(candidate("one"), vec!["e107516f-335d-42f5-b9f4-f71c081c41e7".into()])],
            1,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();
        let mut legacy = serde_json::to_value(manifest).unwrap();
        legacy["schema_version"] = serde_json::json!(1);
        legacy["runs"][0].as_object_mut().unwrap().remove("quality");
        legacy["runs"][0].as_object_mut().unwrap().remove("evidence");
        legacy.as_object_mut().unwrap().remove("repetitions_per_candidate");
        legacy["runs"][0].as_object_mut().unwrap().remove("candidate_ordinal");
        legacy["runs"][0].as_object_mut().unwrap().remove("repetition_index");
        legacy["runs"][0].as_object_mut().unwrap().remove("repetition_total");
        legacy["restore"].as_object_mut().unwrap().remove("profile_fingerprint");
        legacy["restore"].as_object_mut().unwrap().remove("terminal_state");
        legacy["restore"].as_object_mut().unwrap().remove("checks");
        fs::create_dir_all(&root).unwrap();
        let path = root.join(PIPELINE_MANIFEST_NAME);
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();

        let loaded = PipelineManifest::load(path).unwrap();
        assert_eq!(loaded.schema_version, PIPELINE_SCHEMA_VERSION);
        assert!(loaded.runs[0].quality.is_none());
        assert!(loaded.runs[0].evidence.is_none());
        assert_eq!(loaded.repetitions_per_candidate, 1);
        assert_eq!(loaded.runs[0].candidate_ordinal, 1);
        assert_eq!(loaded.runs[0].repetition_index, 1);
        assert_eq!(loaded.runs[0].repetition_total, 1);
        assert!(loaded.restore.checks.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_running_supervisor_becomes_resumable_without_losing_batch_id() {
        let mut manifest = PipelineManifest::create(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            PathBuf::from("/tmp/pipeline"),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({"tun_interface":"Meta"}),
            vec![(candidate("one"), vec!["e107516f-335d-42f5-b9f4-f71c081c41e7".into()])],
            1,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();
        manifest.begin_next_run().unwrap();
        manifest.runs[0].batch_id = Some("e38c26b7-789c-4aa0-b1bb-e3d5916390af".into());
        let committed_at = Utc::now();
        manifest.runs[0].evidence = Some(PipelineRunEvidence {
            profile_activation: Some(PipelineProfileActivation {
                source_profile_uid: Some("profile-zero".into()),
                target_profile_uid: "profile-one".into(),
                requested_at: committed_at,
                profile_already_active: false,
                resumed_from_committed_state: false,
                profile_committed_at: Some(committed_at),
                controller_verified_at: None,
                last_completed_step: PipelineProfileActivationStep::ProfileCommitted,
            }),
            ..PipelineRunEvidence::default()
        });

        assert!(manifest.recover_interrupted_supervisor().unwrap());
        assert_eq!(manifest.state, PipelineState::Interrupted);
        assert_eq!(manifest.current_run_index, None);
        assert_eq!(manifest.runs[0].state, PipelineRunState::Interrupted);
        assert_eq!(
            manifest.runs[0].batch_id.as_deref(),
            Some("e38c26b7-789c-4aa0-b1bb-e3d5916390af")
        );
        assert_eq!(manifest.begin_next_run().unwrap(), Some(0));
        assert_eq!(manifest.runs[0].resume_attempt, 1);
        let activation = manifest.runs[0]
            .evidence
            .as_ref()
            .and_then(|evidence| evidence.profile_activation.as_ref())
            .unwrap();
        assert_eq!(activation.profile_committed_at, Some(committed_at));
        assert_eq!(
            activation.last_completed_step,
            PipelineProfileActivationStep::ProfileCommitted
        );
    }

    #[test]
    fn binds_all_repetitions_to_one_semantic_profile_snapshot() {
        let mut manifest = PipelineManifest::create(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            PathBuf::from("/tmp/pipeline"),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({}),
            vec![(candidate("one"), vec!["one-r1".into(), "one-r2".into()])],
            2,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();

        assert!(manifest.bind_candidate_profile(1, "c".repeat(64), Utc::now()).unwrap());
        assert!(manifest.runs.iter().all(|run| {
            run.profile_fingerprint == "c".repeat(64)
                && run.profile_fingerprint_kind == PIPELINE_FINGERPRINT_SEMANTIC_V2
                && run.profile_bound_at.is_some()
        }));
    }

    #[test]
    fn candidate_failure_skips_remaining_repetitions() {
        let mut manifest = PipelineManifest::create(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            PathBuf::from("/tmp/pipeline"),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({}),
            vec![(candidate("one"), vec!["one-r1".into(), "one-r2".into()])],
            2,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();
        manifest
            .fail_candidate_materialization(
                1,
                PipelineError {
                    code: "SELECTOR_NOT_FOUND".into(),
                    message: "selector missing".into(),
                },
            )
            .unwrap();

        assert_eq!(manifest.runs[0].state, PipelineRunState::Failed);
        assert_eq!(manifest.runs[1].state, PipelineRunState::Skipped);
        assert_eq!(manifest.runs[1].error.as_ref().unwrap().code, "CANDIDATE_BLOCKED");
    }

    #[test]
    fn systemic_failure_skips_every_remaining_run() {
        let mut manifest = PipelineManifest::create(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            PathBuf::from("/tmp/pipeline"),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({}),
            vec![(candidate("one"), vec!["one-r1".into(), "one-r2".into()])],
            2,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();
        let skipped = manifest.skip_remaining_runs(&PipelineError {
            code: "PIPELINE_RUN_FAILED".into(),
            message: "CONTRACT_VALIDATION_FAILED: shared schema mismatch".into(),
        });
        assert_eq!(skipped, 2);
        assert!(manifest.runs.iter().all(|run| run.state == PipelineRunState::Skipped));
        assert!(
            manifest
                .runs
                .iter()
                .all(|run| run.error.as_ref().unwrap().code == "PIPELINE_SYSTEMIC_FAILURE")
        );
    }

    #[test]
    fn loads_schema_four_evidence_without_profile_activation() {
        let manifest = PipelineManifest::create(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            PathBuf::from("/tmp/pipeline"),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({}),
            vec![(candidate("one"), vec!["one-r1".into()])],
            1,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();
        let mut legacy = serde_json::to_value(manifest).unwrap();
        legacy["schema_version"] = serde_json::json!(4);
        legacy["runs"][0]["evidence"] = serde_json::json!({});

        let decoded: PipelineManifest = serde_json::from_value(legacy).unwrap();
        assert!(
            decoded.runs[0]
                .evidence
                .as_ref()
                .is_some_and(|evidence| evidence.profile_activation.is_none())
        );
    }
    #[test]
    fn matrix_runs_follow_repetition_target_candidate_order_and_enforce_barrier() {
        let mut second_target = target();
        second_target.index = 7;
        second_target.url = "https://example.com/second".into();
        second_target.run_label = "second".into();
        let schedule = PipelineSchedule::matrix(
            2,
            2,
            crate::core::traffic_tracer::schedule::PipelineCandidateOrderPolicy::Fixed,
            None,
        )
        .unwrap();
        let mut manifest = PipelineManifest::create_matrix(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            PathBuf::from("/tmp/pipeline"),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target(), second_target],
            serde_json::json!({}),
            vec![candidate("one"), candidate("two")],
            (1..=8).map(|ordinal| format!("matrix-{ordinal}")).collect(),
            2,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
            schedule,
        )
        .unwrap();

        assert_eq!(
            manifest
                .runs
                .iter()
                .map(|run| (run.repetition_index, run.target_index, run.candidate_ordinal,))
                .collect::<Vec<_>>(),
            vec![
                (1, Some(0), 1),
                (1, Some(0), 2),
                (1, Some(7), 1),
                (1, Some(7), 2),
                (2, Some(0), 1),
                (2, Some(0), 2),
                (2, Some(7), 1),
                (2, Some(7), 2),
            ]
        );
        assert!(manifest.begin_next_analysis(1).is_err());

        for expected in 0..4 {
            assert_eq!(manifest.begin_next_capture(1).unwrap(), Some(expected));
            manifest.finish_capture(vec![format!("session-{expected}")]).unwrap();
        }
        assert_eq!(manifest.begin_next_capture(1).unwrap(), None);
        for expected in 0..4 {
            assert_eq!(manifest.begin_next_analysis(1).unwrap(), Some(expected));
            manifest.finish_analysis(PipelineRunState::Completed, None).unwrap();
        }
        assert_eq!(manifest.begin_next_analysis(1).unwrap(), None);
        for expected in 4..8 {
            assert_eq!(manifest.begin_next_capture(2).unwrap(), Some(expected));
            manifest.finish_capture(vec![format!("session-{expected}")]).unwrap();
        }
        assert_eq!(manifest.begin_next_analysis(2).unwrap(), Some(4));
        manifest.schedule_application_retry(1).unwrap();
        assert_eq!(manifest.runs[4].state, PipelineRunState::RetryPending);
        assert_eq!(manifest.runs[4].prior_session_ids, vec!["session-4"]);
        for expected in 5..8 {
            assert_eq!(manifest.begin_next_analysis(2).unwrap(), Some(expected));
            manifest.finish_analysis(PipelineRunState::Completed, None).unwrap();
        }
        assert_eq!(manifest.begin_next_analysis(2).unwrap(), None);
        assert_eq!(manifest.begin_next_capture(2).unwrap(), Some(4));
        manifest.finish_capture(vec!["session-4-retry".into()]).unwrap();
        assert_eq!(manifest.runs[4].application_retry_attempt, 1);
        assert_eq!(manifest.runs[4].session_ids, vec!["session-4-retry"]);
    }

    #[test]
    fn recovery_during_matrix_analysis_preserves_capture_for_resume() {
        let schedule = PipelineSchedule::matrix(
            1,
            1,
            crate::core::traffic_tracer::schedule::PipelineCandidateOrderPolicy::Fixed,
            None,
        )
        .unwrap();
        let mut manifest = PipelineManifest::create_matrix(
            "6ea29d49-4f0e-4f9b-8a88-0ad095c50b78".into(),
            PathBuf::from("/tmp/pipeline"),
            PipelineConfigSnapshot {
                path: PathBuf::from("/tmp/sites.yaml"),
                sha256: "b".repeat(64),
            },
            vec![target()],
            serde_json::json!({}),
            vec![candidate("one")],
            vec!["matrix-one".into()],
            1,
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                profile_fingerprint: None,
                terminal_state: None,
                selections: vec![],
                checks: vec![],
                state: RestoreState::Pending,
                error: None,
            },
            schedule,
        )
        .unwrap();
        manifest.begin_next_capture(1).unwrap();
        manifest.finish_capture(vec!["session-one".into()]).unwrap();
        manifest.begin_next_analysis(1).unwrap();

        assert!(manifest.recover_interrupted_supervisor().unwrap());
        assert_eq!(manifest.runs[0].state, PipelineRunState::Captured);
        assert_eq!(manifest.current_run_index, None);
        assert_eq!(manifest.state, PipelineState::Interrupted);
    }
}
