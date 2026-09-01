import type { PipelineCandidate } from '@/types/traffic-tracer'

export const PIPELINE_QUEUE_STORAGE_KEY = 'traffictracer.pipelineQueue.v1'
export const PIPELINE_MODE_STORAGE_KEY = 'traffictracer.pipelineMode.v1'

export function restoredPipelineCandidates(): PipelineCandidate[] {
  try {
    const value = JSON.parse(
      localStorage.getItem(PIPELINE_QUEUE_STORAGE_KEY) ?? '[]',
    )
    if (!Array.isArray(value)) return []
    const seen = new Set<string>()
    return value.filter((item): item is PipelineCandidate => {
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
        return false
      const identity = `${item.profile_uid}\0${item.selection_group}\0${item.requested_node}`
      if (seen.has(identity)) return false
      seen.add(identity)
      return true
    })
  } catch {
    return []
  }
}
