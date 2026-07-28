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
    const previous = queryClient.getQueryData<ITracingState>(['getTracingState'])
    queryClient.setQueryData(['getTracingState'], (old: ITracingState | undefined) =>
      old ? { ...old, ...payload } : old
    )
    try {
      await patchTracingState(payload)
      await mutateTracing()
    } catch (e) {
      queryClient.setQueryData(['getTracingState'], previous)
      throw e
    }
  }

  return { tracing, mutateTracing, patchTracing, isError, error }
}
