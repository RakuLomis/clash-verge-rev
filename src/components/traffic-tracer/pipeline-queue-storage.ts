import type { PipelineCandidate } from '@/types/traffic-tracer'

export const PIPELINE_QUEUE_STORAGE_KEY = 'traffictracer.pipelineQueue.v2'
const LEGACY_PIPELINE_QUEUE_STORAGE_KEY = 'traffictracer.pipelineQueue.v1'
export const PIPELINE_MODE_STORAGE_KEY = 'traffictracer.pipelineMode.v1'
export const PIPELINE_REPETITIONS_STORAGE_KEY =
  'traffictracer.pipelineRepetitions.v1'
export const PIPELINE_MAX_REPETITIONS = 20

export function restoredPipelineRepetitions(): number {
  const value = Number(localStorage.getItem(PIPELINE_REPETITIONS_STORAGE_KEY))
  return Number.isInteger(value) &&
    value >= 1 &&
    value <= PIPELINE_MAX_REPETITIONS
    ? value
    : 1
}

export function restoredPipelineCandidates(): PipelineCandidate[] {
  try {
    const current = localStorage.getItem(PIPELINE_QUEUE_STORAGE_KEY)
    const legacy =
      current === null
        ? localStorage.getItem(LEGACY_PIPELINE_QUEUE_STORAGE_KEY)
        : null
    const value = JSON.parse(current ?? legacy ?? '[]')
    if (!Array.isArray(value)) return []
    const seen = new Set<string>()
    const restored = value.flatMap((item): PipelineCandidate[] => {
      if (
        typeof item?.profile_uid !== 'string' ||
        typeof item?.profile_fingerprint !== 'string' ||
        !/^[0-9a-f]{64}$/i.test(item.profile_fingerprint) ||
        typeof item?.selection_group !== 'string' ||
        typeof item?.requested_node !== 'string' ||
        !item.profile_uid ||
        !item.selection_group ||
        !item.requested_node
      )
        return []
      const identity = [
        item.profile_uid,
        item.selection_group,
        item.requested_node,
      ].join('::')
      if (seen.has(identity)) return []
      seen.add(identity)
      return [
        {
          profile_uid: item.profile_uid,
          profile_fingerprint: item.profile_fingerprint,
          profile_fingerprint_kind:
            item.profile_fingerprint_kind === 'runtime_semantic_v2'
              ? 'runtime_semantic_v2'
              : 'runtime_bytes_v1',
          recorded_at:
            typeof item.recorded_at === 'string' ? item.recorded_at : null,
          selection_group: item.selection_group,
          requested_node: item.requested_node,
        },
      ]
    })
    if (legacy !== null) {
      localStorage.setItem(PIPELINE_QUEUE_STORAGE_KEY, JSON.stringify(restored))
      localStorage.removeItem(LEGACY_PIPELINE_QUEUE_STORAGE_KEY)
    }
    return restored
  } catch {
    return []
  }
}
