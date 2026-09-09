// Real page orchestration and MUI controls; child workspaces and Worker hooks
// are explicit test doubles. This is not a full native-page soak.
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import type { PropsWithChildren } from 'react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import type { PipelineManifest } from '@/types/traffic-tracer'

const api = vi.hoisted(() => ({
  get: vi.fn(),
  interrupt: vi.fn(),
  resume: vi.fn(),
  heartbeat: vi.fn(),
  cancel: vi.fn(),
  restore: vi.fn(),
  history: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: api.heartbeat }))
vi.mock('@/components/base', () => ({
  BasePage: ({ children }: PropsWithChildren) => <div>{children}</div>,
}))
vi.mock('@/components/traffic-tracer/capture-form', () => ({
  TrafficTracerCaptureForm: () => null,
}))
vi.mock('@/components/traffic-tracer/flow-query-form', () => ({
  TrafficTracerFlowQueryForm: () => null,
}))
vi.mock('@/components/traffic-tracer/pipeline-queue', () => ({
  TrafficTracerPipelineQueue: () => null,
}))
vi.mock('@/components/traffic-tracer/sessions-view', () => ({
  TrafficTracerSessionsView: () => null,
}))
vi.mock('@/components/traffic-tracer/batch-progress', () => ({
  TrafficTracerBatchProgress: () => null,
}))
vi.mock('@/hooks/use-capture-job', () => ({
  useCaptureJob: () => ({
    job: null,
    progressEvents: [],
    startMutation: { isPending: false },
    cancelMutation: { isPending: false },
  }),
}))
vi.mock('@/hooks/use-traffic-tracer-batches', () => ({
  useTrafficTracerBatches: () => ({
    batches: [],
    batchStatus: null,
    startMutation: { isPending: false },
  }),
}))
vi.mock('@/hooks/use-traffic-tracer-worker', () => ({
  useTrafficTracerWorker: () => ({
    environmentQuery: { isFetching: false },
    captureLock: { locked: false },
  }),
}))
vi.mock('@/services/notice-service', () => ({
  showNotice: { success: vi.fn(), error: vi.fn() },
}))
vi.mock('@/services/cmds', () => ({
  getTrafficTracerPipeline: api.get,
  interruptTrafficTracerPipeline: api.interrupt,
  resumeTrafficTracerPipeline: api.resume,
  cancelTrafficTracerPipeline: api.cancel,
  getTrafficTracerBatch: vi.fn(),
  listTrafficTracerPipelines: api.history,
  retryTrafficTracerPipelineRestore: api.restore,
  startTrafficTracerPipeline: vi.fn(),
}))

import TrafficTracerPage from './index'

function manifest(state: PipelineManifest['state']): PipelineManifest {
  return {
    schema_version: 7,
    pipeline_id: 'isolated',
    state,
    stage: 'finished',
    created_at: '2026-09-09T00:00:00Z',
    updated_at: '2026-09-09T00:00:00Z',
    output_root: '/tmp/isolated-page',
    config: { path: '/tmp/sites.yaml', sha256: 'fixture' },
    targets: [],
    execution: {},
    policy: { continue_on_run_failure: true, restore_original_state: true },
    repetitions_per_candidate: 1,
    current_run_index: null,
    runs: [],
    schedule: {
      mode: 'candidate_major',
      candidate_order_policy: 'fixed',
      random_seed: null,
      algorithm_version: 1,
      repetition_candidate_orders: [],
    },
    restore: {
      profile_uid: null,
      selections: [],
      checks: [],
      state: 'not_required',
      error: null,
    },
  }
}
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: Error) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}
async function flush() {
  await act(async () => {
    await Promise.resolve()
  })
}
beforeEach(() => {
  vi.useFakeTimers()
  api.heartbeat.mockResolvedValue(undefined)
  api.history.mockResolvedValue([])
  localStorage.setItem(
    'traffictracer.activePipeline.v1',
    JSON.stringify({
      pipeline_id: 'isolated',
      output_root: '/tmp/isolated-page',
    }),
  )
})
afterEach(() => {
  cleanup()
  localStorage.clear()
  vi.useRealTimers()
  vi.resetAllMocks()
})

it('keeps slow status polling single-flight and disposes timers when leaving', async () => {
  const pending = deferred<PipelineManifest>()
  api.get.mockReturnValue(pending.promise)
  const view = render(<TrafficTracerPage />)
  await act(async () => {
    await vi.advanceTimersByTimeAsync(6000)
  })
  expect(api.get).toHaveBeenCalledTimes(1)
  view.unmount()
  expect(vi.getTimerCount()).toBe(0)
  await act(async () => {
    pending.resolve(manifest('completed'))
  })
  expect(api.heartbeat).toHaveBeenLastCalledWith('tt_ui_heartbeat', {
    active: false,
  })
})

it('does not let an older poll overwrite the interruption result', async () => {
  const old = deferred<PipelineManifest>()
  api.get
    .mockResolvedValueOnce(manifest('running'))
    .mockReturnValue(old.promise)
  api.interrupt.mockResolvedValue(manifest('interrupted'))
  render(<TrafficTracerPage />)
  await flush()
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1000)
  })
  fireEvent.click(screen.getByRole('button', { name: 'Interrupt' }))
  await flush()
  expect(
    screen.getByRole('button', { name: 'Resume pipeline' }),
  ).toBeInTheDocument()
  await act(async () => {
    old.resolve(manifest('running'))
  })
  expect(
    screen.getByRole('button', { name: 'Resume pipeline' }),
  ).toBeInTheDocument()
})

it('does not restore a stale status error after a successful Resume', async () => {
  const old = deferred<PipelineManifest>()
  api.get
    .mockResolvedValueOnce(manifest('interrupted'))
    .mockReturnValue(old.promise)
  api.resume.mockResolvedValue(manifest('running'))
  render(<TrafficTracerPage />)
  await flush()
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1000)
  })
  fireEvent.click(screen.getByRole('button', { name: 'Resume pipeline' }))
  await flush()
  await act(async () => {
    old.reject(new Error('obsolete status failure'))
  })
  expect(screen.queryByText('obsolete status failure')).not.toBeInTheDocument()
})

it.each([
  ['Cancel', 'running', 'cancelled', 'cancel'],
  ['Resume pipeline', 'interrupted', 'running', 'resume'],
  ['Retry restoration', 'restore_failed', 'completed', 'restore'],
] as const)('keeps the %s outcome when an earlier read completes', async (button, before, after, method) => {
  const old = deferred<PipelineManifest>()
  api.get.mockResolvedValueOnce(manifest(before)).mockReturnValue(old.promise)
  api[method].mockResolvedValue(manifest(after))
  render(<TrafficTracerPage />)
  await flush()
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1000)
  })
  fireEvent.click(screen.getByRole('button', { name: button }))
  await flush()
  const outcome = new RegExp(`^${after} · finished`)
  expect(screen.getByText(outcome)).toBeInTheDocument()
  await act(async () => {
    old.resolve(manifest(before))
  })
  expect(screen.getByText(outcome)).toBeInTheDocument()
})

it('pauses polling during an action and resumes after its result', async () => {
  const action = deferred<PipelineManifest>()
  api.get.mockResolvedValue(manifest('running'))
  api.interrupt.mockReturnValue(action.promise)
  render(<TrafficTracerPage />)
  await flush()
  fireEvent.click(screen.getByRole('button', { name: 'Interrupt' }))
  await act(async () => {
    await vi.advanceTimersByTimeAsync(6000)
  })
  expect(api.get).toHaveBeenCalledTimes(1)
  api.get.mockResolvedValue(manifest('interrupted'))
  await act(async () => {
    action.resolve(manifest('interrupted'))
  })
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1000)
  })
  expect(api.get).toHaveBeenCalledTimes(2)
})

it('preserves a current status failure across leaving and returning', async () => {
  api.get
    .mockRejectedValueOnce(new Error('current status failure'))
    .mockResolvedValue(manifest('completed'))
  const view = render(<TrafficTracerPage />)
  await flush()
  expect(screen.getByText('current status failure')).toBeInTheDocument()
  view.unmount()
  render(<TrafficTracerPage />)
  await flush()
  expect(screen.getByText('current status failure')).toBeInTheDocument()
})

it.each([
  ['Interrupt', 'interrupt'],
  ['Cancel', 'cancel'],
] as const)('keeps a failed %s visible after navigation and clears it on successful retry', async (button, method) => {
  api.get.mockResolvedValue(manifest('running'))
  api[method].mockRejectedValueOnce(new Error('stop request failed'))
  const first = render(<TrafficTracerPage />)
  await flush()
  fireEvent.click(screen.getByRole('button', { name: button }))
  await flush()
  expect(screen.getByText('stop request failed')).toBeInTheDocument()
  expect(
    screen.getByText('Pipeline control request failed'),
  ).toBeInTheDocument()
  first.unmount()
  render(<TrafficTracerPage />)
  await flush()
  expect(screen.getByText('stop request failed')).toBeInTheDocument()
  api[method].mockResolvedValue(
    manifest(method === 'cancel' ? 'cancelled' : 'interrupted'),
  )
  fireEvent.click(screen.getByRole('button', { name: button }))
  await flush()
  expect(screen.queryByText('stop request failed')).not.toBeInTheDocument()
})

it('does not accumulate page timers across repeated mounts or apply an old mount response', async () => {
  const old = deferred<PipelineManifest>()
  api.get.mockReturnValue(old.promise)
  for (let index = 0; index < 25; index++) {
    const view = render(<TrafficTracerPage />)
    await flush()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1)
    })
    expect(vi.getTimerCount()).toBe(2)
    view.unmount()
    expect(vi.getTimerCount()).toBe(0)
  }
  api.get.mockResolvedValue(manifest('completed'))
  render(<TrafficTracerPage />)
  await flush()
  await act(async () => {
    old.resolve(manifest('running'))
  })
  expect(screen.getByText(/^completed · finished/)).toBeInTheDocument()
})

it('locks the history selector while a pipeline operation is pending', async () => {
  localStorage.setItem(
    'traffictracer.environmentRequest.v1',
    JSON.stringify({
      output_root: '/tmp/isolated-page',
      tun_interface: 'Meta',
      physical_interface: 'eth0',
      chrome_binary: '/test/chrome',
    }),
  )
  api.history.mockResolvedValue([
    {
      pipeline_id: 'isolated',
      output_root: '/tmp/isolated-page',
      updated_at: '2026-09-09T00:00:00Z',
      state: 'running',
      completed_runs: 0,
      total_runs: 0,
      candidate_count: 1,
      repetitions_per_candidate: 1,
    },
  ])
  api.get.mockResolvedValue(manifest('running'))
  const action = deferred<PipelineManifest>()
  api.interrupt.mockReturnValue(action.promise)
  render(<TrafficTracerPage />)
  await flush()
  const history = screen.getByRole('combobox', {
    name: 'Profile / node pipeline history',
  })
  expect(history).not.toHaveAttribute('aria-disabled', 'true')
  fireEvent.click(screen.getByRole('button', { name: 'Interrupt' }))
  expect(history).toHaveAttribute('aria-disabled', 'true')
  await act(async () => {
    action.resolve(manifest('interrupted'))
  })
  expect(history).not.toHaveAttribute('aria-disabled', 'true')
})
