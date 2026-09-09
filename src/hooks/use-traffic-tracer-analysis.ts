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
  // Consuming the query signal lets React Query cancel/gc abandoned reads even
  // before IPC settles. This does not cancel the backend analysis job itself.
  const queries = useQueries({
    queries: [
      {
        queryKey: ['trafficTracer', 'analysis', sessionId, 'coverage'],
        queryFn: async ({ signal }) => {
          const result = await readTrafficTracerAnalysis<CoverageSummary>(
            sessionId!,
            'coverage_summary',
          )
          if (signal.aborted) throw new DOMException('Cancelled', 'AbortError')
          return result
        },
        enabled: enabled && sessionId !== null,
        retry: false,
        gcTime: 0,
      },
      {
        queryKey: ['trafficTracer', 'analysis', sessionId, 'requests'],
        queryFn: async ({ signal }) => {
          const result = await readTrafficTracerAnalysis<
            AnalysisIndex<RequestIndexRecord>
          >(sessionId!, 'request_index')
          if (signal.aborted) throw new DOMException('Cancelled', 'AbortError')
          return result
        },
        enabled: enabled && sessionId !== null,
        retry: false,
        gcTime: 0,
      },
      {
        queryKey: ['trafficTracer', 'analysis', sessionId, 'connections'],
        queryFn: async ({ signal }) => {
          const result = await readTrafficTracerAnalysis<
            AnalysisIndex<ConnectionIndexRecord>
          >(sessionId!, 'connection_index')
          if (signal.aborted) throw new DOMException('Cancelled', 'AbortError')
          return result
        },
        enabled: enabled && sessionId !== null,
        retry: false,
        gcTime: 0,
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
