import { afterEach, describe, expect, it } from 'vitest'

import {
  PIPELINE_QUEUE_STORAGE_KEY,
  restoredPipelineCandidates,
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
      candidate,
      { ...candidate, requested_node: 'node-two' },
    ])
  })

  it('rejects corrupt or incomplete persisted candidates', () => {
    localStorage.setItem(
      PIPELINE_QUEUE_STORAGE_KEY,
      JSON.stringify([{ profile_uid: 'p', profile_fingerprint: 'bad' }, null]),
    )
    expect(restoredPipelineCandidates()).toEqual([])
  })
})
