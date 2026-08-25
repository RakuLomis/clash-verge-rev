import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, renderHook, waitFor } from '@testing-library/react'
import type { PropsWithChildren } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({ getJob: vi.fn() }))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => vi.fn()),
}))
vi.mock('@/services/cmds', () => ({
  cancelTrafficTracerJob: vi.fn(),
  getTrafficTracerJob: mocks.getJob,
  startTrafficTracerCapture: vi.fn(),
}))

import type { JobProgressEvent, JobSnapshot } from '@/types/traffic-tracer'

import { useCaptureJob } from './use-capture-job'

const snapshot: JobSnapshot = {
  job_id: 'job-persisted',
  kind: 'capture',
  state: 'capturing',
  stage: 'capture.browser',
  progress: 0.4,
  message: 'Launching Chrome',
  cancel_requested: false,
}

afterEach(() => {
  cleanup()
  localStorage.clear()
  vi.clearAllMocks()
})

describe('useCaptureJob progress persistence', () => {
  it('restores the latest timing after the TrafficTracer route remounts', async () => {
    const event: JobProgressEvent = {
      job_id: snapshot.job_id,
      state: snapshot.state,
      stage: snapshot.stage,
      progress: snapshot.progress,
      message: snapshot.message,
      timestamp: '2026-08-25T10:00:00.000Z',
      timing: {
        job_elapsed_ms: 5000,
        stage_elapsed_ms: 2000,
        operation: 'capture.chrome_launch',
        operation_elapsed_ms: 1000,
      },
    }
    localStorage.setItem('traffictracer.activeJobId', snapshot.job_id)
    localStorage.setItem(
      'traffictracer.activeJobProgress.v1',
      JSON.stringify({ job_id: snapshot.job_id, events: [event] }),
    )
    mocks.getJob.mockResolvedValue(snapshot)
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    const wrapper = ({ children }: PropsWithChildren) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    )

    const first = renderHook(() => useCaptureJob(), { wrapper })
    expect(first.result.current.progressEvents).toEqual([event])
    first.unmount()

    const second = renderHook(() => useCaptureJob(), { wrapper })
    await waitFor(() => expect(second.result.current.job).toEqual(snapshot))
    expect(second.result.current.progressEvents).toEqual([event])
  })
})
