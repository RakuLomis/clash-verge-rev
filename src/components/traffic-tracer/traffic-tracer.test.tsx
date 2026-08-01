import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/path', () => ({
  appDataDir: vi.fn(async () => '/tmp/app-data'),
  join: vi.fn(async (...parts: string[]) => parts.join('/')),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('react-router', () => ({ useNavigate: () => vi.fn() }))
vi.mock('@/services/cmds', () => ({
  getNetworkInterfaces: vi.fn(async () => ['mihomo', 'eth0']),
}))

import { formatTrafficTracerCaptureLock } from '@/hooks/use-traffic-tracer-worker'
import type {
  CompleteEnvironmentReport,
  FlowRecord,
  JobSnapshot,
  SessionManifest,
} from '@/types/traffic-tracer'

import { TrafficTracerCaptureForm } from './capture-form'
import { validateCaptureForm } from './capture-form-model'
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
        url: 'not a URL',
        domain: '-invalid.example',
        duration_seconds: 0,
        network: 'all',
        tun_interface: '',
        physical_interface: '',
        output_root: 'relative/output',
        chrome_binary: 'google-chrome',
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
