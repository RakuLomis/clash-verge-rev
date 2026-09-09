import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, renderHook, waitFor } from '@testing-library/react'
import type { PropsWithChildren } from 'react'
import { afterEach, expect, it, vi } from 'vitest'

vi.mock('@/services/cmds', () => ({
  readTrafficTracerAnalysis: vi.fn(async () => ({ items: [] })),
}))

import { readTrafficTracerAnalysis } from '@/services/cmds'

import { useTrafficTracerAnalysis } from './use-traffic-tracer-analysis'

afterEach(cleanup)

it('releases abandoned pending queries before slow IPC finishes', async () => {
  const pending: Array<(value: unknown) => void> = []
  vi.mocked(readTrafficTracerAnalysis).mockImplementation(
    () => new Promise((resolve) => pending.push(resolve)),
  )
  const client = new QueryClient()
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  const view = renderHook(() => useTrafficTracerAnalysis('slow-session'), {
    wrapper,
  })
  try {
    expect(pending).toHaveLength(3)
    view.unmount()
    await waitFor(() => expect(client.getQueryCache().getAll()).toHaveLength(0))
    for (const resolve of pending)
      resolve({ items: [{ marker: 'late-result' }] })
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(client.getQueryCache().getAll()).toHaveLength(0)
  } finally {
    for (const resolve of pending) resolve({ items: [] })
    client.clear()
    vi.mocked(readTrafficTracerAnalysis).mockResolvedValue({ items: [] })
  }
})

it('releases large analysis indexes when the detail view is closed', async () => {
  const client = new QueryClient()
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  const { result, unmount } = renderHook(
    () => useTrafficTracerAnalysis('session-one'),
    { wrapper },
  )
  await waitFor(() => expect(result.current.isLoading).toBe(false))
  expect(client.getQueryCache().getAll()).toHaveLength(3)
  unmount()
  await waitFor(() => expect(client.getQueryCache().getAll()).toHaveLength(0))
  client.clear()
})

it('does not cancel reads while another details consumer still observes them', async () => {
  const pending: Array<(value: unknown) => void> = []
  vi.mocked(readTrafficTracerAnalysis).mockImplementation(
    () => new Promise((resolve) => pending.push(resolve)),
  )
  const client = new QueryClient()
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  const first = renderHook(() => useTrafficTracerAnalysis('shared'), {
    wrapper,
  })
  const second = renderHook(() => useTrafficTracerAnalysis('shared'), {
    wrapper,
  })
  try {
    expect(pending).toHaveLength(3)
    first.unmount()
    expect(client.getQueryCache().getAll()).toHaveLength(3)
    for (const resolve of pending) resolve({ items: [] })
    await waitFor(() => expect(second.result.current.isLoading).toBe(false))
    second.unmount()
    await waitFor(() => expect(client.getQueryCache().getAll()).toHaveLength(0))
  } finally {
    for (const resolve of pending) resolve({ items: [] })
    first.unmount()
    second.unmount()
    client.clear()
    vi.mocked(readTrafficTracerAnalysis).mockResolvedValue({ items: [] })
  }
})

it('does not accumulate cache entries across repeated abandoned detail reads', async () => {
  const pending: Array<(value: unknown) => void> = []
  vi.mocked(readTrafficTracerAnalysis).mockImplementation(
    () => new Promise((resolve) => pending.push(resolve)),
  )
  const client = new QueryClient()
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  try {
    for (let index = 0; index < 12; index++) {
      const view = renderHook(() => useTrafficTracerAnalysis(`slow-${index}`), {
        wrapper,
      })
      view.unmount()
      await waitFor(() =>
        expect(client.getQueryCache().getAll()).toHaveLength(0),
      )
    }
    expect(pending).toHaveLength(36)
    for (const resolve of pending) resolve({ items: [] })
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(client.getQueryCache().getAll()).toHaveLength(0)
  } finally {
    for (const resolve of pending) resolve({ items: [] })
    client.clear()
    vi.mocked(readTrafficTracerAnalysis).mockResolvedValue({ items: [] })
  }
})
