import { useQuery } from '@tanstack/react-query'

import { getTracingState, patchTracingState } from '@/services/cmds'

export const useTracing = () => {
  const { data: tracing, refetch: mutateTracing } = useQuery({
    queryKey: ['getTracingState'],
    queryFn: getTracingState,
    staleTime: 5000,
  })

  const patchTracing = async (payload: ITracingPatch) => {
    await patchTracingState(payload)
    await mutateTracing()
  }

  return { tracing, mutateTracing, patchTracing }
}
