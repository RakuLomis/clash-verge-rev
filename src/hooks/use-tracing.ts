import { useQuery, useQueryClient } from '@tanstack/react-query'

import { getTracingState, patchTracingState } from '@/services/cmds'

export const useTracing = () => {
  const queryClient = useQueryClient()

  const { data: tracing, refetch: mutateTracing, isError, error } = useQuery({
    queryKey: ['getTracingState'],
    queryFn: getTracingState,
    staleTime: Infinity,
  })

  const patchTracing = async (payload: ITracingPatch) => {
    queryClient.setQueryData(['getTracingState'], (old: ITracingState | undefined) =>
      old ? { ...old, ...payload } : old
    )
    await patchTracingState(payload)
    await mutateTracing()
  }

  return { tracing, mutateTracing, patchTracing, isError, error }
}
