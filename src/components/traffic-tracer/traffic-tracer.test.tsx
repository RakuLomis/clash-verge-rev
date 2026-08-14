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
  SessionSummary,
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

const session: SessionSummary = {
  schema_version: 1,
  session_id: 'session-one',
  job_id: 'job-one',
  state: 'completed',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:30Z',
  session_dir: '/tmp/session-one',
  target: { url: 'https://example.com/', domain: 'example.com' },
  artifact_count: 0,
  warning_count: 0,
  quality_state: 'passed',
  capture_global_quality_state: 'passed',
  coverage: null,
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

  it('defaults new captures to standard analysis storage', async () => {
    render(<TrafficTracerCaptureForm onDiagnose={vi.fn()} onSubmit={vi.fn()} />)

    expect(
      await screen.findByLabelText('Browser cache policy'),
    ).toHaveTextContent('Cold (recommended)')
    expect(await screen.findByLabelText('Analysis storage')).toHaveTextContent(
      'Standard (recommended)',
    )
    expect(
      screen.getByText(
        'Standard keeps raw captures and indexes; derived connection PCAPs can be generated by re-analysis.',
      ),
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
          pcap_split_mode: 'none',
          cache_mode: 'cold',
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
        playback: null,
        options: {
          capture_packets: true,
          collect_cdp: true,
          collect_netlog: true,
          analyze_after_capture: true,
          headless: false,
          pcap_split_mode: 'none',
          cache_mode: 'cold',
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
      trace_snapshot: {
        source: 'mihomo_barrier',
        trace_count: 1,
        late_event_count: 3,
        traces: [
          {
            source: 'mihomo_barrier',
            cutoff_event_seq: 42,
            barrier_ts: '2026-08-12T00:00:00Z',
            barrier_session_id: 'session-one',
            late_event_count: 3,
            max_observed_event_seq: 45,
          },
        ],
      },
      storage: {
        capture_bytes: 1073741824,
        raw_packet_capture_bytes: 536870912,
        netlog_bytes: 400000000,
        mihomo_trace_bytes: 100000000,
        capture_metadata_bytes: 36870912,
        analysis_result_bytes_before_summary: 1000,
        compression: 'none',
      },
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
          playback: {
            provider: 'youtube' as const,
            ad_policy: 'click_visible_skip' as const,
            desired_primary_seconds: 10,
          },
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
      playback: {
        provider: 'youtube',
        ad_policy: 'click_visible_skip',
        desired_primary_seconds: 10,
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
        application_protocol: 'unknown',
        attempted_protocols: ['QUIC'],
        pre_flow: flow.pre_flow,
        post_flow: null,
        terminal: {
          status: 'dial_error',
          stage: 'dial',
          error: 'connection timed out',
          error_class: 'ipv4_timeout',
          bytes_up: 0,
          bytes_down: 0,
          duration_ms: 30000,
        },
        shared: true,
        sharing: {
          request_multiplexed: true,
          post_flow_shared: false,
          outer_connection_reused: false,
        },
        egress: {
          mode: 'proxy',
          policy: 'Youtube',
          selection_chain: ['Youtube', 'Proxy group', 'Vless node'],
          selected_node: 'Vless node',
          selected_type: 'Vless',
          evidence: 'mihomo_trace_and_session_proxy_snapshot',
        },

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
    const localConnectionId = `conn-${'4'.repeat(32)}`
    requests.push({
      request_id: 'local-probe',
      url: 'http://localhost.weixin.qq.com/',
      resource_type: 'Document',
      relation: 'cross_site',
      connection_id: localConnectionId,
      candidate_connection_ids: [localConnectionId],
      network_observation: 'local_endpoint',
      attribution: {
        status: 'matched',
        method: 'netlog_socket',
        confidence: 1,
        evidence: ['transport_request_ids'],
      },
    })
    connections.push({
      ...connections[0],
      connection_id: localConnectionId,
      application_protocol: 'h2',
      attempted_protocols: [],
      request_ids: ['local-probe'],
      primary_url: 'http://localhost.weixin.qq.com/',
      urls: ['http://localhost.weixin.qq.com/'],
      terminal: {
        ...connections[0].terminal!,
        error: 'connection refused',
        error_class: 'connection_refused',
      },
      sharing: {
        request_multiplexed: false,
        post_flow_shared: false,
        outer_connection_reused: false,
      },
      egress: {
        mode: 'direct',
        policy: null,
        selection_chain: [],
        selected_node: null,
        selected_type: null,
        evidence: 'local_endpoint',
      },
      match: {
        status: 'matched',
        method: 'netlog_socket',
        confidence: 1,
        evidence: ['local_endpoint'],
        candidates: [],
      },
    })
    const summary: CoverageSummary = {
      coverage_source: 'v2_indexes',
      match_method_counts: { endpoint_time: 1 },
      quality_state: 'degraded',
      capture_global_quality_state: 'degraded',
      trace_snapshot: {
        source: 'mihomo_barrier',
        trace_count: 1,
        late_event_count: 3,
        traces: [
          {
            source: 'mihomo_barrier',
            cutoff_event_seq: 42,
            barrier_ts: '2026-08-12T00:00:00Z',
            barrier_session_id: 'session-one',
            late_event_count: 3,
            max_observed_event_seq: 45,
          },
        ],
      },
      storage: {
        capture_bytes: 1073741824,
        raw_packet_capture_bytes: 536870912,
        netlog_bytes: 400000000,
        mihomo_trace_bytes: 100000000,
        capture_metadata_bytes: 36870912,
        analysis_result_bytes_before_summary: 1000,
        compression: 'none',
      },
      warnings: [
        {
          code: 'EGRESS_DIAL_FAILED',
          count: 1,
          message: 'The egress dial failed.',
          scope: 'page_attributed',
          severity: 'warning',
          affects_page_quality: true,
        },
        {
          code: 'POST_FLOW_UNAVAILABLE',
          count: 2,
          message: 'Background flows have no post tuple.',
          scope: 'capture_global',
          severity: 'warning',
          affects_page_quality: false,
        },
      ],
      quality: {
        request_attribution: {
          eligible: 2,
          matched: 2,
          ambiguous: 0,
          unmatched: 0,
        },
        transport_correlation: {
          total: 1,
          matched: 0,
          ambiguous: 1,
          unmatched: 0,
        },
        egress_establishment: {
          total: 2,
          established: 0,
          failed_before_socket: 1,
          unavailable: 0,
          not_applicable_local_endpoint: 1,
        },
        pcap_extraction: {
          requested: true,
          total: 2,
          applicable: 1,
          pre_success: 1,
          post_success: 0,
          complete_pairs: 0,
          post_not_applicable: 1,
        },
        capture_global: {
          logical_flows: {
            total: 3,
            with_post_flow: 1,
            missing_post_flow: 2,
            errors: 1,
            not_applicable_local_endpoint: 1,
          },
        },
      },
      coverage: {
        browser_requests: {
          total: 3,
          matched: 2,
          ambiguous: 0,
          unmatched: 0,
          non_network: 1,
        },
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
        page_attributed: {
          browser_requests: {
            total: 3,
            matched: 2,
            ambiguous: 0,
            unmatched: 0,
            non_network: 1,
          },
          transport_connections: {
            total: 1,
            matched: 0,
            ambiguous: 1,
            unmatched: 0,
          },
          logical_flows: {
            total: 1,
            with_post_flow: 0,
            shared: 1,
            missing_post_flow: 1,
          },
          unmatched_reasons: { multiple_candidates: 1, missing_post_flow: 1 },
        },
        capture_global: {
          core_logical_flows: {
            total: 3,
            with_post_flow: 1,
            shared: 1,
            missing_post_flow: 2,
          },
          unmatched_reasons: { missing_post_flow: 2 },
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
    expect(screen.getAllByText('dial_error · dial')).toHaveLength(2)
    expect(screen.getByText('2 candidates')).toBeInTheDocument()
    expect(
      screen.getByText('Youtube → Proxy group → Vless node'),
    ).toBeInTheDocument()
    expect(screen.getByText('request multiplexing')).toBeInTheDocument()
    expect(screen.getByText(/1 non-network/)).toBeInTheDocument()
    expect(
      screen.getByText(
        /Page flows: 0 socket established.*0 explicit no-socket.*1 unexpected missing/,
      ),
    ).toBeInTheDocument()
    expect(
      screen.getByText(
        /Capture-global flows: 1 socket established.*1 local N\/A.*1 unexpected missing/,
      ),
    )
    expect(
      screen.getByText(
        'Trace snapshot: barrier · 1 trace · 3 late events excluded',
      ),
    )
    expect(
      screen.getByText(
        'Storage: 1.00 GiB capture · 512.0 MiB raw PCAP · compression none',
      ),
    ).toBeInTheDocument()
    expect(
      screen.getByText('cdn.example · ipv4_timeout: 1'),
    ).toBeInTheDocument()
    expect(
      screen.queryByText(/localhost\.weixin\.qq\.com · connection_refused/),
    ).not.toBeInTheDocument()
    expect(
      screen.getByText('Page analysis integrity: degraded'),
    ).toBeInTheDocument()
    expect(
      screen.getByText('Capture-global analysis integrity: degraded'),
    ).toBeInTheDocument()
    expect(screen.getByText('POST_FLOW_UNAVAILABLE: 2')).toBeInTheDocument()
    expect(screen.getByText('EGRESS_DIAL_FAILED: 1')).toBeInTheDocument()
    expect(
      screen.getByText(
        /Egress: 0\/1 applicable established · 1 dial failed · 1 local N\/A/,
      ),
    ).toBeInTheDocument()
    expect(
      screen.getByText(
        /PCAP pairs: 0\/1 applicable complete.*1 unavailable.*1 local post N\/A/,
      ),
    ).toBeInTheDocument()
    expect(
      screen.getByText(
        /Local endpoint probes: 1.*post-proxy flow not applicable/,
      ),
    ).toBeInTheDocument()
    expect(
      screen.getByText('app: unknown · attempted QUIC'),
    ).toBeInTheDocument()
  })

  it('renders explicit rejected egress without treating post-flow as missing', () => {
    const connection: ConnectionIndexRecord = {
      connection_id: `conn-${'9'.repeat(32)}`,
      protocol: 'tcp',
      pre_flow: flow.pre_flow,
      post_flow: null,
      terminal: {
        status: 'closed',
        stage: 'reject',
        error: '',
        bytes_up: 0,
        bytes_down: 0,
        duration_ms: 1,
      },
      shared: false,
      egress: {
        mode: 'unknown',
        outcome: 'rejected',
        policy: 'Taobao',
        selection_chain: ['Taobao', 'REJECT'],
        selected_node: 'REJECT',
        selected_type: 'Reject',
        evidence: 'mihomo_trace',
      },
      request_ids: [],
      primary_url: 'https://taobao.com/',
      urls: ['https://taobao.com/'],
      match: {
        status: 'matched',
        method: 'netlog_socket',
        confidence: 1,
        evidence: ['mihomo_connection_id'],
        candidates: [],
      },
    }

    render(
      <TrafficTracerConnectionResults
        requests={[]}
        connections={[connection]}
      />,
    )

    expect(
      screen.getByText(/Explicit no-socket egress outcomes: 1/),
    ).toBeInTheDocument()
    expect(screen.getByText('Not applicable · rejected')).toBeInTheDocument()
    expect(screen.getByText('rejected')).toBeInTheDocument()
    expect(screen.getByText('Taobao → REJECT')).toBeInTheDocument()
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
