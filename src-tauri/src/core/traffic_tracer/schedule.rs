use std::collections::HashSet;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const PIPELINE_SCHEDULE_ALGORITHM_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineScheduleMode {
    #[default]
    CandidateMajor,
    RepetitionTargetCandidate,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineCandidateOrderPolicy {
    #[default]
    Fixed,
    BalancedSeeded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineSchedule {
    pub mode: PipelineScheduleMode,
    pub candidate_order_policy: PipelineCandidateOrderPolicy,
    #[serde(default)]
    pub random_seed: Option<u64>,
    pub algorithm_version: u32,
    #[serde(default)]
    pub repetition_candidate_orders: Vec<Vec<u16>>,
}

impl Default for PipelineSchedule {
    fn default() -> Self {
        Self {
            mode: PipelineScheduleMode::CandidateMajor,
            candidate_order_policy: PipelineCandidateOrderPolicy::Fixed,
            random_seed: None,
            algorithm_version: PIPELINE_SCHEDULE_ALGORITHM_VERSION,
            repetition_candidate_orders: Vec::new(),
        }
    }
}

impl PipelineSchedule {
    pub fn matrix(
        repetitions: u16,
        candidate_count: usize,
        candidate_order_policy: PipelineCandidateOrderPolicy,
        random_seed: Option<u64>,
    ) -> Result<Self> {
        if repetitions == 0 {
            bail!("pipeline schedule requires at least one repetition");
        }
        let candidate_count =
            u16::try_from(candidate_count).map_err(|_| anyhow::anyhow!("pipeline candidate count exceeds u16"))?;
        if candidate_count == 0 {
            bail!("pipeline schedule requires at least one candidate");
        }
        if candidate_order_policy == PipelineCandidateOrderPolicy::BalancedSeeded && random_seed.is_none() {
            bail!("balanced seeded schedule requires random_seed");
        }

        let repetition_candidate_orders = match candidate_order_policy {
            PipelineCandidateOrderPolicy::Fixed => {
                let order = (1..=candidate_count).collect::<Vec<_>>();
                vec![order; usize::from(repetitions)]
            }
            PipelineCandidateOrderPolicy::BalancedSeeded => {
                balanced_orders(repetitions, candidate_count, random_seed.unwrap_or_default())
            }
        };
        let schedule = Self {
            mode: PipelineScheduleMode::RepetitionTargetCandidate,
            candidate_order_policy,
            random_seed,
            algorithm_version: PIPELINE_SCHEDULE_ALGORITHM_VERSION,
            repetition_candidate_orders,
        };
        schedule.validate(repetitions, usize::from(candidate_count))?;
        Ok(schedule)
    }

    pub fn validate(&self, repetitions: u16, candidate_count: usize) -> Result<()> {
        if self.algorithm_version != PIPELINE_SCHEDULE_ALGORITHM_VERSION {
            bail!("unsupported pipeline schedule algorithm version");
        }
        match self.mode {
            PipelineScheduleMode::CandidateMajor => {
                if !self.repetition_candidate_orders.is_empty() {
                    bail!("candidate-major schedule must not contain matrix orders");
                }
            }
            PipelineScheduleMode::RepetitionTargetCandidate => {
                if self.repetition_candidate_orders.len() != usize::from(repetitions) {
                    bail!("matrix schedule repetition order count is inconsistent");
                }
                let expected = (1..=u16::try_from(candidate_count)
                    .map_err(|_| anyhow::anyhow!("pipeline candidate count exceeds u16"))?)
                    .collect::<HashSet<_>>();
                for order in &self.repetition_candidate_orders {
                    if order.len() != candidate_count || order.iter().copied().collect::<HashSet<_>>() != expected {
                        bail!("matrix schedule candidate order is not a permutation");
                    }
                }
                if self.candidate_order_policy == PipelineCandidateOrderPolicy::BalancedSeeded
                    && self.random_seed.is_none()
                {
                    bail!("balanced seeded schedule requires random_seed");
                }
            }
        }
        Ok(())
    }
}

fn balanced_orders(repetitions: u16, candidate_count: u16, seed: u64) -> Vec<Vec<u16>> {
    let count = usize::from(candidate_count);
    let mut orders = Vec::with_capacity(usize::from(repetitions));
    for repetition in 0..usize::from(repetitions) {
        let block = repetition / count;
        let offset = repetition % count;
        let mut base = (1..=candidate_count).collect::<Vec<_>>();
        seeded_shuffle(&mut base, seed ^ (block as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        base.rotate_left(offset);
        orders.push(base);
    }
    orders
}

fn seeded_shuffle(values: &mut [u16], seed: u64) {
    let mut state = seed;
    for index in (1..values.len()).rev() {
        let random = splitmix64(&mut state);
        values.swap(index, random as usize % (index + 1));
    }
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_schedule_is_reproducible_and_position_balanced() {
        let schedule =
            PipelineSchedule::matrix(3, 3, PipelineCandidateOrderPolicy::BalancedSeeded, Some(20260905)).unwrap();
        let repeated =
            PipelineSchedule::matrix(3, 3, PipelineCandidateOrderPolicy::BalancedSeeded, Some(20260905)).unwrap();

        assert_eq!(schedule, repeated);
        for candidate in 1..=3 {
            for position in 0..3 {
                assert_eq!(
                    schedule
                        .repetition_candidate_orders
                        .iter()
                        .filter(|order| order[position] == candidate)
                        .count(),
                    1
                );
            }
        }
    }

    #[test]
    fn fixed_matrix_schedule_keeps_queue_order() {
        let schedule = PipelineSchedule::matrix(3, 3, PipelineCandidateOrderPolicy::Fixed, None).unwrap();

        assert_eq!(
            schedule.repetition_candidate_orders,
            vec![vec![1, 2, 3], vec![1, 2, 3], vec![1, 2, 3]]
        );
    }

    #[test]
    fn rejects_unseeded_or_inconsistent_matrix_schedule() {
        assert!(PipelineSchedule::matrix(2, 3, PipelineCandidateOrderPolicy::BalancedSeeded, None,).is_err());
        let mut schedule = PipelineSchedule::matrix(2, 3, PipelineCandidateOrderPolicy::Fixed, None).unwrap();
        schedule.repetition_candidate_orders[1] = vec![1, 1, 2];
        assert!(schedule.validate(2, 3).is_err());
    }
}
