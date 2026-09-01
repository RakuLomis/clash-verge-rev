use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const PIPELINE_SCHEMA_VERSION: u32 = 1;
pub const PIPELINE_MANIFEST_NAME: &str = "pipeline-manifest.json";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineState {
    Created,
    Validating,
    Running,
    Interrupted,
    Completed,
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
            Self::Completed | Self::CompletedWithErrors | Self::Failed | Self::Cancelled | Self::RestoreFailed
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
    RunningBatch,
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
    pub selections: Vec<PipelineSelection>,
    pub state: RestoreState,
    pub error: Option<PipelineError>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineRun {
    pub ordinal: usize,
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
        candidates: Vec<(String, PipelineCandidate)>,
        policy: PipelinePolicy,
        restore: PipelineRestore,
    ) -> Result<Self> {
        if targets.is_empty() || candidates.is_empty() {
            bail!("pipeline requires at least one target and candidate");
        }
        if !output_root.is_absolute() || !config.path.is_absolute() {
            bail!("pipeline paths must be absolute");
        }
        if config.sha256.len() != 64 || !config.sha256.chars().all(|value| value.is_ascii_hexdigit()) {
            bail!("pipeline config sha256 must be SHA-256");
        }
        let mut identities = HashSet::new();
        let mut runs = Vec::with_capacity(candidates.len());
        for (index, (run_id, candidate)) in candidates.into_iter().enumerate() {
            candidate.validate()?;
            let identity = (
                candidate.profile_uid.clone(),
                candidate.selection_group.clone(),
                candidate.requested_node.clone(),
            );
            if !identities.insert(identity) {
                bail!("pipeline candidates must be unique by profile, selector and node");
            }
            runs.push(PipelineRun {
                ordinal: index + 1,
                run_id: run_id.clone(),
                profile_uid: candidate.profile_uid,
                profile_fingerprint: candidate.profile_fingerprint,
                selection_group: candidate.selection_group,
                requested_node: candidate.requested_node,
                state: PipelineRunState::Pending,
                stage: PipelineStage::Queued,
                resolved_chain: Vec::new(),
                resolved_leaf: None,
                expected_protocol: String::new(),
                observed_protocol: String::new(),
                batch_id: None,
                output_path: output_root.join("runs").join(format!(
                    "{:03}_{}",
                    index + 1,
                    &run_id[..12.min(run_id.len())]
                )),
                error: None,
                resume_attempt: 0,
                started_at: None,
                completed_at: None,
            });
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
            current_run_index: None,
            runs,
            restore,
        })
    }

    pub fn persist(&self) -> Result<PathBuf> {
        fs::create_dir_all(&self.output_root).context("create pipeline output directory")?;
        let path = self.output_root.join(PIPELINE_MANIFEST_NAME);
        let temporary = self.output_root.join(format!(".{PIPELINE_MANIFEST_NAME}.tmp"));
        let mut bytes = serde_json::to_vec_pretty(self).context("serialize pipeline manifest")?;
        bytes.push(b'\n');
        fs::write(&temporary, bytes).context("write pipeline manifest checkpoint")?;
        fs::rename(&temporary, &path).context("commit pipeline manifest checkpoint")?;
        Ok(path)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let data = fs::read(path.as_ref()).context("read pipeline manifest")?;
        let manifest: Self = serde_json::from_slice(&data).context("decode pipeline manifest")?;
        if manifest.schema_version != PIPELINE_SCHEMA_VERSION {
            bail!("unsupported pipeline manifest schema version");
        }
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
            self.state = if self.runs.iter().any(|run| {
                matches!(
                    run.state,
                    PipelineRunState::Failed | PipelineRunState::Degraded | PipelineRunState::Skipped
                )
            }) {
                PipelineState::CompletedWithErrors
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
                PipelinePolicy {
                    continue_on_run_failure: true,
                    restore_original_state: true,
                },
                PipelineRestore {
                    profile_uid: Some("profile-one".into()),
                    selections: vec![],
                    state: RestoreState::Pending,
                    error: None,
                },
            )
        };
        let manifest = build(vec![
            ("e107516f-335d-42f5-b9f4-f71c081c41e7".into(), candidate("one")),
            ("e38c26b7-789c-4aa0-b1bb-e3d5916390af".into(), candidate("two")),
        ])
        .unwrap();
        assert_eq!(
            manifest.runs.iter().map(|run| run.ordinal).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(
            build(vec![
                ("e107516f-335d-42f5-b9f4-f71c081c41e7".into(), candidate("same")),
                ("e38c26b7-789c-4aa0-b1bb-e3d5916390af".into(), candidate("same"))
            ])
            .is_err()
        );
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
                ("e107516f-335d-42f5-b9f4-f71c081c41e7".into(), candidate("one")),
                ("e38c26b7-789c-4aa0-b1bb-e3d5916390af".into(), candidate("two")),
            ],
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                selections: vec![],
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
            vec![("e107516f-335d-42f5-b9f4-f71c081c41e7".into(), candidate("one"))],
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                selections: vec![],
                state: RestoreState::Pending,
                error: None,
            },
        )
        .unwrap();
        manifest.begin_next_run().unwrap();
        let path = manifest.persist().unwrap();
        assert_eq!(PipelineManifest::load(path).unwrap(), manifest);
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
            vec![("e107516f-335d-42f5-b9f4-f71c081c41e7".into(), candidate("one"))],
            PipelinePolicy {
                continue_on_run_failure: true,
                restore_original_state: true,
            },
            PipelineRestore {
                profile_uid: None,
                selections: vec![],
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
