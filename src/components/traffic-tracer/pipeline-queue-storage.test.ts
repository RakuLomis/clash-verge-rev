import { afterEach, describe, expect, it } from 'vitest'

import {
  PIPELINE_QUEUE_STORAGE_KEY,
  PIPELINE_REPETITIONS_STORAGE_KEY,
  restoredPipelineCandidates,
  restoredPipelineRepetitions,
} from './pipeline-queue-storage'

afterEach(() => localStorage.clear())

describe('pipeline queue persistence', () => {
  it('restores valid tuples in order and removes exact duplicates', () => {
    const candidate = {
      profile_uid: 'profile-one',
      profile_fingerprint: 'a'.repeat(64),
      selection_group: 'GLOBAL',
      requested_node: 'node-one',
    }
    localStorage.setItem(
      PIPELINE_QUEUE_STORAGE_KEY,
      JSON.stringify([
        candidate,
        candidate,
        { ...candidate, requested_node: 'node-two' },
      ]),
    )
    expect(restoredPipelineCandidates()).toEqual([
      {
        ...candidate,
        profile_fingerprint_kind: 'runtime_bytes_v1',
        recorded_at: null,
      },
      {
        ...candidate,
        requested_node: 'node-two',
        profile_fingerprint_kind: 'runtime_bytes_v1',
        recorded_at: null,
      },
    ])
  })

  it('migrates legacy v1 candidates as unchecked byte snapshots', () => {
    localStorage.setItem(
      'traffictracer.pipelineQueue.v1',
      JSON.stringify([
        {
          profile_uid: 'legacy-profile',
          profile_fingerprint: 'b'.repeat(64),
          selection_group: 'GLOBAL',
          requested_node: 'legacy-node',
        },
      ]),
    )

    expect(restoredPipelineCandidates()[0]).toMatchObject({
      profile_uid: 'legacy-profile',
      profile_fingerprint_kind: 'runtime_bytes_v1',
      recorded_at: null,
    })
    expect(localStorage.getItem(PIPELINE_QUEUE_STORAGE_KEY)).not.toBeNull()
    expect(localStorage.getItem('traffictracer.pipelineQueue.v1')).toBeNull()
  })

  it('rejects corrupt or incomplete persisted candidates', () => {
    localStorage.setItem(
      PIPELINE_QUEUE_STORAGE_KEY,
      JSON.stringify([{ profile_uid: 'p', profile_fingerprint: 'bad' }, null]),
    )
    expect(restoredPipelineCandidates()).toEqual([])
  })

  it('restores only bounded integer repetition counts', () => {
    expect(restoredPipelineRepetitions()).toBe(1)
    localStorage.setItem(PIPELINE_REPETITIONS_STORAGE_KEY, '3')
    expect(restoredPipelineRepetitions()).toBe(3)
    for (const invalid of ['0', '21', '1.5', 'bad']) {
      localStorage.setItem(PIPELINE_REPETITIONS_STORAGE_KEY, invalid)
      expect(restoredPipelineRepetitions()).toBe(1)
    }
  })
})
