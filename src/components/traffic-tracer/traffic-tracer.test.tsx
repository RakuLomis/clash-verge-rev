import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/path', () => ({
  appDataDir: vi.fn(async () => '/tmp/app-data'),
  join: vi.fn(async (...parts: string[]) => parts.join('/')),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('react-router', () => ({ useNavigate: () => vi.fn() }))
vi.mock('./session-detail', () => ({
  TrafficTracerSessionDetail: ({ sessionId }: { sessionId: string | null }) =>
    sessionId ? <div>Opened analysis for {sessionId}</div> : null,
}))
vi.mock('@/services/cmds', () => ({
  getNetworkInterfaces: vi.fn(async () => ['mihomo', 'eth0']),
  getVergeConfig: vi.fn(async () => ({
    traffic_tracer_output_root: '/tmp/persisted sessions',
  })),
}))

import { formatTrafficTracerCaptureLock } from '@/hooks/use-traffic-tracer-worker'
import type {
  BatchStatusResult,
  CompleteEnvironmentReport,
  ConnectionIndexRecord,
  CoverageSummary,
  FlowRecord,
  JobSnapshot,
  SessionManifest,
  RequestIndexRecord,
} from '@/types/traffic-tracer'

import { TrafficTracerBatchProgress } from './batch-progress'
import { TrafficTracerCaptureForm } from './capture-form'
import {
  applyTargetConfigEntry,
  batchRequestFromDraft,
  captureRequestFromDraft,
  defaultCaptureFormDraft,
  selectedTargetsInConfigOrder,
  suggestCaptureInterfaces,
  validateCaptureForm,
} from './capture-form-model'
import { TrafficTracerConnectionResults } from './connection-results'
import { TrafficTracerEnvironmentCard } from './environment-card'
import { TrafficTracerFlowTable } from './flow-table'
import { TrafficTracerJobProgress } from './job-progress'
import { TrafficTracerSessionCard } from './session-card'

afterEach(() => {
  cleanup()
  localStorage.clear()
})

const blockingEnvironment: CompleteEnvironmentReport = {
  level: 'blocking',
  ok: false,
  checks: [
    {
      code: 'CORE_NOT_TRAFFIC_TRACER',
      ok: false,
      severity: 'error',
      message: 'raw core message',
      remediation: 'raw remediation',
      details: {},
    },
  ],
  integration: {
    current_core: 'verge-mihomo',
    tun_enabled: true,
    service_available: true,
    configured_tun_device: '',
    automatic_tun_device: 'Meta',
    capture_tun_interface: 'mihomo',
    worker: { state: 'stopped' },
  },
}

const session: SessionManifest = {
  schema_version: 1,
  session_id: 'session-one',
  job_id: 'job-one',
  state: 'completed',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:30Z',
  session_dir: '/tmp/session-one',
  target: { url: 'https://example.com/', domain: 'example.com' },
  component_versions: {
    traffictracer: { version: '1', commit: 'tt' },
    mihomo: { version: '1', commit: 'mihomo' },
    clash_verge_rev: { version: '1', commit: 'ui' },
    worker_api: 1,
  },
  artifacts: [],
  warnings: [],
}

const flow: FlowRecord = {
  schema_version: 1,
  session_id: 'session-one',
  flow_id: 'flow-one',
  protocol: 'tcp',
  pre_flow: {
    network: 'tcp',
    src_ip: '10.0.0.2',
    src_port: 50123,
    dst_ip: '93.184.216.34',
    dst_port: 443,
    dst_host: 'example.com',
    complete: true,
    source: 'mihomo',
    scope: 'logical',
    shared: false,
  },
  post_flow: null,
  shared: false,
  match: {
    status: 'unmatched',
    confidence: 0,
    candidate_count: 0,
    reason: 'no outer connection',
  },
  request_ids: [],
  url: 'https://example.com/',
}

describe('TrafficTracer Complete workspace', () => {
  it('uses the persisted Verge workspace and does not mirror it to localStorage', async () => {
    localStorage.setItem(
      'traffictracer.captureForm.v1',
      JSON.stringify({ output_root: '/tmp/legacy browser root' }),
    )
    render(<TrafficTracerCaptureForm onDiagnose={vi.fn()} onSubmit={vi.fn()} />)

    expect(
      await screen.findByDisplayValue('/tmp/persisted sessions'),
    ).toBeInTheDocument()
    await waitFor(() => {
      const stored = JSON.parse(
        localStorage.getItem('traffictracer.captureForm.v1') ?? '{}',
      ) as Record<string, unknown>
      expect(stored).not.toHaveProperty('output_root')
    })
  })

  it('restores the accepted workspace after a switch error', async () => {
    const view = render(
      <TrafficTracerCaptureForm onDiagnose={vi.fn()} onSubmit={vi.fn()} />,
    )
    const output = await screen.findByLabelText('Session output directory')
    await userEvent.clear(output)
    await userEvent.type(output, '/tmp/rejected root')

    view.rerender(
      <TrafficTracerCaptureForm
        diagnosticError={new Error('SESSION_ROOT_BUSY')}
        onDiagnose={vi.fn()}
        onSubmit={vi.fn()}
      />,
    )

    expect(
      await screen.findByDisplayValue('/tmp/persisted sessions'),
    ).toBeInTheDocument()
  })

  it('keeps Start capture disabled while environment diagnostics are blocking', async () => {
    const diagnosticRequest = {
      tun_interface: 'mihomo',
      physical_interface: 'eth0',
      chrome_binary: '/usr/bin/chromium',
      output_root: '/tmp/traffictracer-sessions',
    }
    localStorage.setItem(
      'traffictracer.captureForm.v1',
      JSON.stringify({
        url: 'https://example.com/',
        domain: 'example.com',
        duration_seconds: 30,
        network: 'all',
        ...diagnosticRequest,
        options: {
          capture_packets: true,
          collect_cdp: true,
          collect_netlog: true,
          analyze_after_capture: true,
          headless: false,
        },
      }),
    )
    const onSubmit = vi.fn()
    render(
      <TrafficTracerCaptureForm
        environment={blockingEnvironment}
        diagnosticRequest={diagnosticRequest}
        onDiagnose={vi.fn()}
        onSubmit={onSubmit}
      />,
    )

    const start = screen.getByRole('button', { name: 'Start capture' })
    expect(start).toBeDisabled()
    expect(onSubmit).not.toHaveBeenCalled()
  })

  it('shows blocking diagnostics and dispatches their remediation target', async () => {
    const onRemediate = vi.fn()
    render(
      <TrafficTracerEnvironmentCard
        report={blockingEnvironment}
        onRemediate={onRemediate}
      />,
    )

    expect(
      screen.getByTestId('traffic-tracer-environment-card'),
    ).toHaveAttribute('data-environment-level', 'blocking')
    expect(
      screen.getByText(
        'The running core does not provide TrafficTracer capabilities.',
      ),
    ).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Fix' }))
    expect(onRemediate).toHaveBeenCalledWith(
      'core',
      blockingEnvironment.checks[0],
    )
  })

  it('returns stable validation codes for an invalid capture form', () => {
    expect(
      validateCaptureForm({
        target_mode: 'manual',
        config_path: '',
        config_sha256: '',
        selected_target_index: null,
        url: 'not a URL',
        domain: '-invalid.example',
        duration_seconds: 0,
        network: 'all',
        tun_interface: '',
        physical_interface: '',
        output_root: 'relative/output',
        chrome_binary: 'google-chrome',
        wait_load_timeout: 30,
        run_label: 'all',
        page_type: 'capture',
        options: {
          capture_packets: true,
          collect_cdp: true,
          collect_netlog: true,
          analyze_after_capture: true,
          headless: false,
        },
      }),
    ).toEqual({
      url: 'url',
      domain: 'domain',
      duration_seconds: 'duration',
      tun_interface: 'tunInterface',
      physical_interface: 'physicalInterface',
      output_root: 'output',
      chrome_binary: 'chrome',
    })
  })

  it('applies a normalized YAML target and preserves its provenance', () => {
    const preview = {
      schema_version: 1 as const,
      config_path: '/tmp/sites.yaml',
      sha256: 'a'.repeat(64),
      warnings: [],
      suggested_output_root: null,
      targets: [
        {
          index: 3,
          url: 'https://example.com/path',
          domain: 'example.com',
          duration_seconds: 12,
          network: 'all' as const,
          run_label: 'browser',
          wait_load_timeout: 45,
          page_type: 'browser',
        },
      ],
    }
    const draft = applyTargetConfigEntry(
      {
        ...defaultCaptureFormDraft,
        tun_interface: 'mihomo',
        physical_interface: 'eth0',
        output_root: '/tmp/sessions',
        chrome_binary: '/usr/bin/chromium',
      },
      preview,
      preview.targets[0],
    )

    expect(validateCaptureForm(draft)).toEqual({})
    expect(captureRequestFromDraft(draft)).toMatchObject({
      url: 'https://example.com/path',
      domain: 'example.com',
      duration_seconds: 12,
      network: 'all',
      wait_load_timeout: 45,
      run_label: 'browser',
      page_type: 'browser',
      target_source: {
        mode: 'config',
        config_path: '/tmp/sites.yaml',
        config_sha256: 'a'.repeat(64),
        target_index: 3,
      },
    })
  })

  it('does not guess when multiple TUN interfaces are present', () => {
    expect(suggestCaptureInterfaces(['Meta', 'Meta0', 'eth0'])).toEqual({
      tun: '',
      physical: 'eth0',
    })
    expect(suggestCaptureInterfaces(['Meta', 'eth0'])).toEqual({
      tun: 'Meta',
      physical: 'eth0',
    })
  })

  it('keeps a selected YAML subset in file order and identifies duplicates by index', () => {
    const duplicate = {
      domain: 'example.com',
      url: 'https://example.com/video',
      duration_seconds: 10,
      network: 'all' as const,
      run_label: 'video',
      wait_load_timeout: 30,
      page_type: 'video',
    }
    const preview = {
      schema_version: 1 as const,
      config_path: '/tmp/sites.yaml',
      sha256: 'b'.repeat(64),
      warnings: [],
      suggested_output_root: null,
      targets: [
        { index: 4, ...duplicate },
        {
          index: 8,
          ...duplicate,
          domain: 'cdn.example.com',
          url: 'https://cdn.example.com/a',
        },
        { index: 12, ...duplicate },
      ],
    }
    const selected = new Set([12, 4])

    expect(
      selectedTargetsInConfigOrder(preview, selected).map(
        (target) => target.index,
      ),
    ).toEqual([4, 12])
    expect(
      batchRequestFromDraft(
        {
          ...defaultCaptureFormDraft,
          tun_interface: 'Meta',
          physical_interface: 'eth0',
          output_root: '/tmp/sessions',
          chrome_binary: '/usr/bin/chromium',
          options: {
            ...defaultCaptureFormDraft.options,
            analyze_after_capture: false,
          },
        },
        preview,
        selected,
      ),
    ).toMatchObject({
      config_sha256: 'b'.repeat(64),
      targets: [{ index: 4 }, { index: 12 }],
      options: { analyze_after_capture: true },
      fail_fast: true,
    })
  })

  it('renders capture group progress and opens a child analysis', async () => {
    const status: BatchStatusResult = {
      batch: {
        schema_version: 1,
        batch_id: 'batch-one',
        state: 'running',
        stage: 'analysis',
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:01:00Z',
        output_root: '/tmp/sessions',
        config: { path: '/tmp/sites.yaml', sha256: 'c'.repeat(64) },
        targets: [
          {
            index: 0,
            domain: 'example.com',
            url: 'https://example.com/',
            duration_seconds: 10,
            network: 'all',
            run_label: 'all',
            wait_load_timeout: 30,
            page_type: 'main-page',
          },
          {
            index: 1,
            domain: 'example.org',
            url: 'https://example.org/',
            duration_seconds: 10,
            network: 'all',
            run_label: 'all',
            wait_load_timeout: 30,
            page_type: 'video-play1',
          },
        ],
        current_index: 1,
        children: [
          {
            target_index: 0,
            state: 'completed',
            session_id: 'session-one',
            error: null,
          },
          {
            target_index: 1,
            state: 'running',
            session_id: null,
            error: null,
          },
        ],
        fail_fast: true,
        cancel_requested: false,
        resume: { attempt: 0, next_index: 1, resumed_at: null },
      },
      job: null,
    }
    const onCancel = vi.fn()
    render(
      <TrafficTracerBatchProgress
        status={status}
        workspaceRoot="/tmp/sessions"
        onCancel={onCancel}
        onResume={vi.fn()}
      />,
    )

    expect(screen.getByText(/Target 2\/2 · analysis/)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Analysis' }))
    expect(
      screen.getByText('Opened analysis for session-one'),
    ).toBeInTheDocument()
    await userEvent.click(
      screen.getByRole('button', { name: 'Cancel capture group' }),
    )
    expect(onCancel).toHaveBeenCalledOnce()
  })

  it('requires confirmation before cancelling an active Job', async () => {
    const job: JobSnapshot = {
      job_id: 'job-active',
      kind: 'capture',
      state: 'capturing',
      stage: 'capturing',
      progress: 0.5,
      message: 'capturing packets',
      cancel_requested: false,
    }
    const onCancel = vi.fn()
    render(<TrafficTracerJobProgress job={job} onCancel={onCancel} />)

    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(onCancel).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: 'Cancel Job' }))
    expect(onCancel).toHaveBeenCalledWith(
      'Cancelled from the TrafficTracer workspace.',
    )
  })

  it('disables Session analysis while capture locking is active', () => {
    render(
      <TrafficTracerSessionCard
        session={session}
        analysisBlocked
        onOpenDirectory={vi.fn()}
        onAnalyze={vi.fn()}
        onView={vi.fn()}
      />,
    )

    expect(screen.getByRole('button', { name: 'Analyze again' })).toBeDisabled()
  })

  it('shows a missing post-proxy tuple and selects its Flow row', async () => {
    const onSelect = vi.fn()
    render(<TrafficTracerFlowTable flows={[flow]} onSelect={onSelect} />)

    expect(screen.getByText('No complete post-proxy tuple')).toBeInTheDocument()
    await userEvent.click(screen.getByTestId('flow-row-session-one-flow-one'))
    expect(onSelect).toHaveBeenCalledWith(flow)
  })

  it('shows shared requests, ambiguity, and missing post flow by connection', () => {
    const connectionId = `conn-${'1'.repeat(32)}`
    const requests: RequestIndexRecord[] = ['one', 'two'].map((id) => ({
      request_id: id,
      url: `https://cdn.example/${id}.js`,
      resource_type: 'Script',
      relation: 'cross_site',
      connection_id: connectionId,
      candidate_connection_ids: [connectionId],
      attribution: {
        status: 'matched',
        method: 'netlog_socket',
        confidence: 1,
        evidence: ['transport_request_ids'],
      },
    }))
    const connections: ConnectionIndexRecord[] = [
      {
        connection_id: connectionId,
        protocol: 'tcp',
        pre_flow: flow.pre_flow,
        post_flow: null,
        terminal: {
          status: 'dial_error',
          stage: 'dial',
          error: 'connection timed out',
          bytes_up: 0,
          bytes_down: 0,
          duration_ms: 30000,
        },
        shared: true,
        request_ids: ['one', 'two'],
        primary_url: 'https://cdn.example/one.js',
        urls: ['https://cdn.example/one.js', 'https://cdn.example/two.js'],
        match: {
          status: 'ambiguous',
          method: 'endpoint_time',
          confidence: 0.78,
          evidence: ['top_score_tie'],
          unmatched_reason: 'multiple_candidates',
          candidates: [
            {
              connection_id: `conn-${'2'.repeat(32)}`,
              score: 0.78,
              evidence: [],
            },
            {
              connection_id: `conn-${'3'.repeat(32)}`,
              score: 0.78,
              evidence: [],
            },
          ],
        },
      },
    ]
    const summary: CoverageSummary = {
      coverage_source: 'v2_indexes',
      match_method_counts: { endpoint_time: 1 },
      coverage: {
        browser_requests: { total: 2, matched: 2, ambiguous: 0, unmatched: 0 },
        transport_connections: {
          total: 1,
          matched: 0,
          ambiguous: 1,
          unmatched: 0,
        },
        core_logical_flows: {
          total: 1,
          with_post_flow: 0,
          shared: 1,
          missing_post_flow: 1,
        },
        unmatched_reasons: { multiple_candidates: 1, missing_post_flow: 1 },
      },
    }
    render(
      <TrafficTracerConnectionResults
        summary={summary}
        requests={requests}
        connections={connections}
      />,
    )
    expect(screen.getAllByText('https://cdn.example/one.js')).toHaveLength(2)
    expect(screen.getByText('https://cdn.example/two.js')).toBeInTheDocument()
    expect(screen.getByText(/2 request\(s\).*2 URL\(s\)/)).toBeInTheDocument()
    expect(screen.getByText('dial_error · dial')).toBeInTheDocument()
    expect(screen.getByText('2 candidates')).toBeInTheDocument()
    expect(
      screen.getByText(/Core flows: 0\/1 with post flow/),
    ).toBeInTheDocument()
  })

  it('explains unavailable connection artifacts for a legacy Session', () => {
    render(
      <TrafficTracerConnectionResults
        requests={[]}
        connections={[]}
        unavailable
      />,
    )
    expect(
      screen.getByText(/legacy Session has no connection-centric/),
    ).toBeInTheDocument()
  })

  it('formats a localized capture lock with its Job identifier', () => {
    expect(
      formatTrafficTracerCaptureLock(
        {
          locked: true,
          job_id: 'job-lock',
          reason: 'TrafficTracer capture is active',
        },
        '捕获任务正在运行。',
        (id) => `任务 ${id}`,
      ),
    ).toBe('捕获任务正在运行。 (任务 job-lock)')
  })
})
