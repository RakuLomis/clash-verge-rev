import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, renderHook, waitFor } from '@testing-library/react'
import type { PropsWithChildren } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type {
  BatchManifest,
  BatchStatusResult,
  JobSnapshot,
} from '@/types/traffic-tracer'

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  get: vi.fn(),
  resume: vi.fn(),
}))

vi.mock('@/services/cmds', () => ({
  cancelTrafficTracerBatch: vi.fn(),
  interruptTrafficTracerBatch: vi.fn(),
  getTrafficTracerBatch: mocks.get,
  listTrafficTracerBatches: mocks.list,
  resumeTrafficTracerBatch: mocks.resume,
  startTrafficTracerBatch: vi.fn(),
}))

import { useTrafficTracerBatches } from './use-traffic-tracer-batches'

const manifest = (state: BatchManifest['state'], attempt: number) => ({
  schema_version: 1 as const,
  batch_id: 'batch-one',
  state,
  stage: state === 'running' ? ('capture' as const) : ('finished' as const),
  created_at: '2026-08-18T12:29:04.000Z',
  updated_at:
    state === 'running'
      ? '2026-08-19T10:00:01.000Z'
      : '2026-08-18T12:30:00.000Z',
  output_root: '/tmp/captures',
  config: { path: '/tmp/sites.yaml', sha256: 'abc' },
  targets: [],
  current_index: state === 'running' ? 1 : 0,
  children: [],
  fail_fast: false,
  cancel_requested: false,
  resume: { attempt, next_index: 1, resumed_at: null },
})

const status = (
  state: BatchManifest['state'],
  attempt: number,
): BatchStatusResult => ({ batch: manifest(state, attempt), job: null })

const resumedJob: JobSnapshot = {
  job_id: 'batch-one',
  kind: 'batch',
  state: 'capturing',
  stage: 'capture',
  progress: 0,
  message: 'Resuming batch',
  cancel_requested: false,
}

afterEach(() => {
  cleanup()
  localStorage.clear()
  vi.clearAllMocks()
})

describe('useTrafficTracerBatches Resume transition', () => {
  it('keeps polling through a stale terminal response until resumed state appears', async () => {
    localStorage.setItem(
      'traffictracer.activeBatchId:%2Ftmp%2Fcaptures',
      'batch-one',
    )
    mocks.list.mockResolvedValue({
      batches: [manifest('failed', 0)],
      corrupt: [],
    })
    mocks.get
      .mockResolvedValueOnce(status('failed', 0))
      .mockResolvedValueOnce(status('failed', 0))
      .mockResolvedValue(status('running', 1))
    mocks.resume.mockResolvedValue(resumedJob)

    const client = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })
    const wrapper = ({ children }: PropsWithChildren) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    )
    const { result } = renderHook(
      () => useTrafficTracerBatches('/tmp/captures'),
      { wrapper },
    )

    await waitFor(() =>
      expect(result.current.batchStatus?.batch.state).toBe('failed'),
    )
    await act(async () => {
      await result.current.resumeBatch()
    })

    await waitFor(() => expect(mocks.get).toHaveBeenCalledTimes(2))
    expect(result.current.resuming).toBe(true)
    expect(
      localStorage.getItem('traffictracer.activeBatchId:%2Ftmp%2Fcaptures'),
    ).toBe('batch-one')

    await waitFor(
      () => expect(result.current.batchStatus?.batch.state).toBe('running'),
      { timeout: 2500 },
    )
    expect(mocks.get.mock.calls.length).toBeGreaterThanOrEqual(3)
    expect(
      localStorage.getItem('traffictracer.activeBatchId:%2Ftmp%2Fcaptures'),
    ).toBe('batch-one')
  })

  it('does not auto-select an old failed or interrupted batch', async () => {
    mocks.list.mockResolvedValue({
      batches: [manifest('failed', 0), manifest('interrupted', 0)],
      corrupt: [],
    })
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    const wrapper = ({ children }: PropsWithChildren) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    )
    const { result } = renderHook(
      () => useTrafficTracerBatches('/tmp/captures'),
      { wrapper },
    )

    await waitFor(() => expect(result.current.listQuery.isSuccess).toBe(true))
    expect(result.current.batchStatus).toBeUndefined()
    expect(result.current.activeBatchId).toBeNull()
    expect(mocks.get).not.toHaveBeenCalled()
  })
  it('loads a history batch only after explicit selection', async () => {
    mocks.list.mockResolvedValue({
      batches: [manifest('completed', 0)],
      corrupt: [],
    })
    mocks.get.mockResolvedValue(status('completed', 0))
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    const wrapper = ({ children }: PropsWithChildren) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    )
    const { result } = renderHook(
      () => useTrafficTracerBatches('/tmp/captures'),
      { wrapper },
    )

    await waitFor(() => expect(result.current.listQuery.isSuccess).toBe(true))
    expect(result.current.batchStatus).toBeUndefined()

    act(() => result.current.selectBatch('batch-one'))

    await waitFor(() =>
      expect(result.current.batchStatus?.batch.state).toBe('completed'),
    )
    expect(result.current.activeBatchId).toBeNull()
    expect(result.current.viewedBatchId).toBe('batch-one')
    expect(
      localStorage.getItem('traffictracer.viewedBatchId:%2Ftmp%2Fcaptures'),
    ).toBe('batch-one')
    expect(mocks.get).toHaveBeenCalledWith('batch-one')
  })
})
