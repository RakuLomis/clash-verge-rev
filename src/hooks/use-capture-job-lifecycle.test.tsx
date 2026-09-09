import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, renderHook } from '@testing-library/react'
import type { PropsWithChildren } from 'react'
import { afterEach, expect, it, vi } from 'vitest'

import type { JobProgressEvent, JobSnapshot } from '@/types/traffic-tracer'

const api = vi.hoisted(() => ({ listen: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: api.listen }))
vi.mock('@/hooks/use-traffic-tracer-worker', () => ({
  trafficTracerCaptureLockKey: ['lock'],
}))
vi.mock('@/services/cmds', () => ({
  getTrafficTracerJob: vi.fn(async () => ({
    job_id: 'one',
    state: 'analyzing',
    progress: 0,
  })),
  cancelTrafficTracerJob: vi.fn(),
  startTrafficTracerCapture: vi.fn(),
}))
import {
  mergeTrafficTracerProgress,
  trafficTracerJobKey,
  useCaptureJob,
} from './use-capture-job'

it('late progress cannot overwrite an authoritative terminal snapshot', () => {
  for (const state of [
    'completed',
    'failed',
    'cancelled',
    'interrupted',
  ] as const) {
    const snapshot = { job_id: 'one', state, progress: 1 } as JobSnapshot
    const progress = {
      job_id: 'one',
      state: 'analyzing',
      progress: 0.5,
    } as JobProgressEvent
    expect(mergeTrafficTracerProgress(snapshot, progress)).toBe(snapshot)
  }
})

afterEach(() => {
  cleanup()
  localStorage.clear()
  vi.restoreAllMocks()
  vi.resetAllMocks()
})
function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  return { client, wrapper }
}
async function flush() {
  await act(async () => {
    await Promise.resolve()
  })
}

it('cleans successful registrations even when one of the five fails', async () => {
  vi.spyOn(console, 'error').mockImplementation(() => {})
  const unlisten = vi.fn()
  api.listen.mockImplementation(async (name: string) => {
    if (name.endsWith('job-state')) throw new Error('injected listener failure')
    return unlisten
  })
  const { client, wrapper } = setup()
  const hook = renderHook(() => useCaptureJob('one'), { wrapper })
  await flush()
  expect(api.listen).toHaveBeenCalledTimes(5)
  hook.unmount()
  expect(unlisten).toHaveBeenCalledTimes(4)
  client.clear()
})

it('ignores queued progress after unmount instead of changing cache or storage', async () => {
  let progress!: (event: { payload: JobProgressEvent }) => void
  api.listen.mockImplementation(
    async (name: string, handler: typeof progress) => {
      if (name.endsWith('job-progress')) progress = handler
      return vi.fn()
    },
  )
  const { client, wrapper } = setup()
  const hook = renderHook(() => useCaptureJob('one'), { wrapper })
  await flush()
  hook.unmount()
  const snapshot = {
    job_id: 'one',
    state: 'completed',
    progress: 1,
  } as JobSnapshot
  client.setQueryData(trafficTracerJobKey('one'), snapshot)
  await act(async () => {
    progress({
      payload: {
        job_id: 'one',
        state: 'analyzing',
        stage: 'analyzing',
        progress: 0.2,
        message: 'late event',
        timestamp: '2026-09-09T00:00:00Z',
      },
    })
  })
  expect(client.getQueryData(trafficTracerJobKey('one'))).toEqual(snapshot)
  expect(localStorage.getItem('traffictracer.activeJobProgress.v1')).toBeNull()
  client.clear()
})

it('disposes registrations that resolve after unmount', async () => {
  const completions: Array<(unlisten: () => void) => void> = []
  api.listen.mockImplementation(
    () => new Promise((resolve) => completions.push(resolve)),
  )
  const { client, wrapper } = setup()
  const hook = renderHook(() => useCaptureJob('one'), { wrapper })
  hook.unmount()
  const unlisten = vi.fn()
  await act(async () => {
    completions.forEach((resolve) => resolve(unlisten))
  })
  expect(unlisten).toHaveBeenCalledTimes(5)
  client.clear()
})
