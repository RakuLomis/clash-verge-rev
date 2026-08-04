import { useQueries } from '@tanstack/react-query'

import { readTrafficTracerAnalysis } from '@/services/cmds'
import type {
  AnalysisIndex,
  ConnectionIndexRecord,
  CoverageSummary,
  RequestIndexRecord,
} from '@/types/traffic-tracer'

export function useTrafficTracerAnalysis(
  sessionId: string | null,
  enabled = true,
) {
  const queries = useQueries({
    queries: [
      {
        queryKey: ['trafficTracer', 'analysis', sessionId, 'coverage'],
        queryFn: () =>
          readTrafficTracerAnalysis<CoverageSummary>(
            sessionId!,
            'coverage_summary',
          ),
        enabled: enabled && sessionId !== null,
        retry: false,
      },
      {
        queryKey: ['trafficTracer', 'analysis', sessionId, 'requests'],
        queryFn: () =>
          readTrafficTracerAnalysis<AnalysisIndex<RequestIndexRecord>>(
            sessionId!,
            'request_index',
          ),
        enabled: enabled && sessionId !== null,
        retry: false,
      },
      {
        queryKey: ['trafficTracer', 'analysis', sessionId, 'connections'],
        queryFn: () =>
          readTrafficTracerAnalysis<AnalysisIndex<ConnectionIndexRecord>>(
            sessionId!,
            'connection_index',
          ),
        enabled: enabled && sessionId !== null,
        retry: false,
      },
    ],
  })
  return {
    summary: queries[0].data,
    requests: queries[1].data?.items ?? [],
    connections: queries[2].data?.items ?? [],
    isLoading: queries.some((query) => query.isLoading),
    unavailable:
      queries.every((query) => query.isError) &&
      !queries.some((query) => query.data),
  }
}
