import { useQuery } from '@tanstack/react-query'

import { trafficTracerFlowsKey } from '@/hooks/use-traffic-tracer-sessions'
import { queryTrafficTracerFlows } from '@/services/cmds'
import type { FlowQueryRequest } from '@/types/traffic-tracer'

export const trafficTracerFlowQueryKey = (request: FlowQueryRequest) =>
  [...trafficTracerFlowsKey, request.session_id, request] as const

export function useFlowQuery(request: FlowQueryRequest | null, enabled = true) {
  const flowQuery = useQuery({
    queryKey: request
      ? trafficTracerFlowQueryKey(request)
      : [...trafficTracerFlowsKey, 'none'],
    queryFn: () => queryTrafficTracerFlows(request!),
    enabled: enabled && request !== null,
  })

  return {
    result: flowQuery.data,
    matches: flowQuery.data?.items ?? [],
    total: flowQuery.data?.total ?? 0,
    flowQuery,
  }
}
