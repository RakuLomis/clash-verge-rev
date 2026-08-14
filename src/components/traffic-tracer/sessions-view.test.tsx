import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  resolveScope: vi.fn(),
  listScoped: vi.fn(),
  openFolder: vi.fn(),
  previewSplit: vi.fn(),
  startSplit: vi.fn(),
  getJob: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => vi.fn()),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mocks.openFolder }))
vi.mock('@/services/cmds', () => ({
  resolveTrafficTracerSessionScope: mocks.resolveScope,
  listTrafficTracerScopedSessions: mocks.listScoped,
  openTrafficTracerSessionDirectory: vi.fn(),
  startTrafficTracerAnalysis: vi.fn(),
  previewTrafficTracerPacketSplit: mocks.previewSplit,
  startTrafficTracerPacketSplit: mocks.startSplit,
  getTrafficTracerJob: mocks.getJob,
  cancelTrafficTracerJob: vi.fn(),
}))
vi.mock('@/services/notice-service', () => ({
  showNotice: { success: vi.fn(), error: vi.fn() },
}))
vi.mock('./session-detail', () => ({
  TrafficTracerSessionDetail: () => null,
}))

import type { SessionScope, SessionSummary } from '@/types/traffic-tracer'

import { TrafficTracerSessionsView } from './sessions-view'

const scope: SessionScope = {
  scope_id: '20260805-110256-685',
  display_name: '20260805-110256-685',
  directory: '/tmp/sessions/20260805-110256-685',
  kind: 'capture_group',
  created_at: '2026-08-05T11:02:56.685Z',
  exists: true,
}

const session: SessionSummary = {
  schema_version: 2,
  session_id: '6a877821-2019-4e5f-8297-39e2d77e08a1',
  job_id: '2f746e31-d62a-4e1c-a919-3f88ecde31c2',
  state: 'completed',
  created_at: '2026-08-05T11:02:56.685Z',
  updated_at: '2026-08-05T11:03:56.685Z',
  session_dir: `${scope.directory}/example.com/main-page__https_example.com`,
  target: { url: 'https://example.com/', domain: 'example.com' },
  artifact_count: 0,
  warning_count: 0,
  quality_state: 'passed',
  capture_global_quality_state: 'passed',
  coverage: null,
}

function renderView(
  props: { activeJobId?: string | null; activeBatchId?: string | null } = {},
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  return render(
    <QueryClientProvider client={client}>
      <TrafficTracerSessionsView
        enabled
        workspaceRoot="/tmp/sessions"
        {...props}
      />
    </QueryClientProvider>,
  )
}

afterEach(() => {
  cleanup()
  localStorage.clear()
  vi.clearAllMocks()
})

describe('TrafficTracer scoped Sessions', () => {
  const splitPreview = {
    scope,
    total: 1,
    counts: { unsplit: 1 },
    missing_only: 1,
    repair_incomplete: 0,
    sessions: [],
    corrupt: [],
  }

  it('does not list root Sessions while idle and no folder is selected', () => {
    renderView()

    expect(
      screen.getByText(/No capture folder is selected/),
    ).toBeInTheDocument()
    expect(mocks.resolveScope).not.toHaveBeenCalled()
    expect(mocks.listScoped).not.toHaveBeenCalled()
  })

  it('lets an idle user choose one timestamp folder manually', async () => {
    mocks.openFolder.mockResolvedValue(scope.directory)
    mocks.resolveScope.mockResolvedValue(scope)
    mocks.listScoped.mockResolvedValue({
      scope,
      sessions: [session],
      corrupt: [],
      offset: 0,
      limit: 8,
      total: 1,
      has_more: false,
    })
    mocks.previewSplit.mockResolvedValue(splitPreview)

    renderView()
    await screen.getByRole('button', { name: 'Choose folder' }).click()

    await waitFor(() => {
      expect(mocks.resolveScope).toHaveBeenCalledWith({ path: scope.directory })
    })
    expect(await screen.findByText('example.com')).toBeInTheDocument()
    expect(screen.queryByText('Current capture')).not.toBeInTheDocument()
    expect(
      await screen.findByRole('button', { name: 'Split missing (1)' }),
    ).toBeEnabled()
  })

  it('clears a stale packet split Job instead of polling forever', async () => {
    localStorage.setItem('traffictracer.packetSplitJobId', 'stale-job')
    mocks.getJob.mockRejectedValue(new Error('Job not found'))

    renderView()

    await waitFor(() => {
      expect(mocks.getJob).toHaveBeenCalledWith('stale-job')
      expect(localStorage.getItem('traffictracer.packetSplitJobId')).toBeNull()
    })
  })

  it('does not preview packet splits while a capture batch is active', async () => {
    mocks.resolveScope.mockResolvedValue(scope)
    mocks.listScoped.mockResolvedValue({
      scope,
      sessions: [session],
      corrupt: [],
      offset: 0,
      limit: 8,
      total: 1,
      has_more: false,
    })

    renderView({ activeBatchId: 'active-batch' })

    await screen.findByText('Current capture')
    expect(mocks.previewSplit).not.toHaveBeenCalled()
  })

  it('automatically selects and lists only the active capture folder', async () => {
    mocks.resolveScope.mockResolvedValue(scope)
    mocks.listScoped.mockResolvedValue({
      scope,
      sessions: [session],
      corrupt: [],
      offset: 0,
      limit: 8,
      total: 1,
      has_more: false,
    })

    renderView({ activeJobId: session.job_id })

    await waitFor(() => {
      expect(mocks.resolveScope).toHaveBeenCalledWith({
        job_id: session.job_id,
      })
    })
    expect(await screen.findByText('Current capture')).toBeInTheDocument()
    expect(await screen.findByText('example.com')).toBeInTheDocument()
    expect(mocks.listScoped).toHaveBeenCalledWith(scope.scope_id, 0, 8)
  })
})
