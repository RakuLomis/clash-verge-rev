import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, renderHook, waitFor } from '@testing-library/react'
import type { PropsWithChildren } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({ list: vi.fn() }))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => vi.fn()),
}))
vi.mock('@/services/cmds', () => ({
  getTrafficTracerSession: vi.fn(),
  listTrafficTracerScopedSessions: vi.fn(),
  listTrafficTracerSessions: mocks.list,
  startTrafficTracerAnalysis: vi.fn(),
}))

import { useAllTrafficTracerSessions } from './use-traffic-tracer-sessions'

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

describe('TrafficTracer lazy Session history', () => {
  it('does not load all history until the cross-Session query requests it', async () => {
    mocks.list.mockResolvedValue({
      sessions: [],
      corrupt: [],
      offset: 0,
      limit: 100,
      total: 0,
      has_more: false,
    })
    const { result } = renderHook(
      () => useAllTrafficTracerSessions(false, '/tmp/captures'),
      { wrapper: wrapper() },
    )

    expect(mocks.list).not.toHaveBeenCalled()
    await act(async () => {
      await result.current.sessionsQuery.refetch()
    })
    await waitFor(() => expect(mocks.list).toHaveBeenCalledWith(0, 100))
  })
})
