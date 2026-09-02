use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const PIPELINE_SCHEMA_VERSION: u32 = 4;
const PIPELINE_MIN_SCHEMA_VERSION: u32 = 1;
pub const PIPELINE_MANIFEST_NAME: &str = "pipeline-manifest.json";
pub const PIPELINE_AGGREGATE_NAME: &str = "pipeline-aggregate.json";
pub const PIPELINE_MAX_REPETITIONS: u16 = 20;

fn default_repetitions() -> u16 {
    1
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
    Restoring,
    Finished,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunState {
    Pending,
    Running,
    Completed,
    Degraded,
    Failed,
    Interrupted,
    Skipped,
    Cancelled,
}

impl PipelineRunState {
    pub fn terminal(self) -> bool {
        !matches!(self, Self::Pending | Self::Running)
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineRunEvidence {
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
    pub run_id: String,
    pub profile_uid: String,
    pub profile_fingerprint: String,
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
                    profile_fingerprint: candidate.profile_fingerprint.clone(),
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
            let mut aggregate = PipelineCandidateAggregate {
                candidate_ordinal,
                profile_uid: first.profile_uid.clone(),
                selection_group: first.selection_group.clone(),
                requested_node: first.requested_node.clone(),
                repetitions_planned: self.repetitions_per_candidate,
                repetitions_terminal: 0,
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
                aggregate.repetitions_terminal += usize::from(run.state.terminal());
                match run.state {
                    PipelineRunState::Completed => aggregate.completed += 1,
                    PipelineRunState::Degraded => aggregate.degraded += 1,
                    PipelineRunState::Failed | PipelineRunState::Skipped => aggregate.failed += 1,
                    PipelineRunState::Interrupted => aggregate.interrupted += 1,
                    PipelineRunState::Cancelled => aggregate.cancelled += 1,
                    PipelineRunState::Pending | PipelineRunState::Running => {}
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
            schema_version: 1,
            pipeline_id: self.pipeline_id.clone(),
            updated_at: self.updated_at,
            repetitions_per_candidate: self.repetitions_per_candidate,
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
        if manifest.schema_version < 4 {
            manifest.repetitions_per_candidate = 1;
            for run in &mut manifest.runs {
                run.candidate_ordinal =
                    u16::try_from(run.ordinal).context("legacy pipeline candidate ordinal overflow")?;
                run.repetition_index = 1;
                run.repetition_total = 1;
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
        if self.current_run_index.is_some() {
            self.finish_run(
                PipelineRunState::Interrupted,
                Some(PipelineError {
                    code: "PIPELINE_SUPERVISOR_RESTARTED".into(),
                    message: "The application stopped while this pipeline run was active; resume can continue its Batch checkpoint.".into(),
                }),
            )?;
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
        if run.state == PipelineRunState::Interrupted {
            run.resume_attempt += 1;
        }
        run.state = PipelineRunState::Running;
        run.stage = PipelineStage::ActivatingProfile;
        run.error = None;
        run.quality = None;
        run.evidence = None;
        run.started_at.get_or_insert_with(Utc::now);
        self.state = PipelineState::Running;
        self.stage = PipelineStage::ActivatingProfile;
        self.current_run_index = Some(index);
        self.updated_at = Utc::now();
        Ok(Some(index))
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(node: &str) -> PipelineCandidate {
        PipelineCandidate {
            profile_uid: "profile-one".into(),
            profile_fingerprint: "a".repeat(64),
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
    }
}
