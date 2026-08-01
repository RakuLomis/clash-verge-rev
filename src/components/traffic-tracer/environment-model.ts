import type {
  CompleteEnvironmentReport,
  DiagnosticCheck,
  EnvironmentRequest,
} from '@/types/traffic-tracer'

export type EnvironmentItemState = 'ready' | 'warning' | 'error' | 'unknown'

export type EnvironmentRemediationTarget =
  | 'core'
  | 'tun'
  | 'tun-service'
  | 'interfaces'
  | 'capture-tools'
  | 'chrome'
  | 'output'
  | 'diagnostics'

export interface EnvironmentSummaryItem {
  id:
    | 'core'
    | 'controller'
    | 'tun'
    | 'tun-interface'
    | 'physical-interface'
    | 'capture-tools'
    | 'chrome'
    | 'output'
  label: string
  value: string
  state: EnvironmentItemState
}

export interface TrafficTracerEnvironmentCardProps {
  report?: CompleteEnvironmentReport
  request?: EnvironmentRequest
  loading?: boolean
  error?: unknown
  onRetry?: () => void
  onRemediate?: (
    target: EnvironmentRemediationTarget,
    check: DiagnosticCheck,
  ) => void
}

const checkPrefixes: Record<EnvironmentSummaryItem['id'], string[]> = {
  core: ['CORE_NOT_TRAFFIC_TRACER'],
  controller: [
    'CORE_ENDPOINT_',
    'CORE_TRACING_',
    'CORE_CAPABILITY_',
    'CORE_READY',
  ],
  tun: ['TUN_DISABLED', 'TUN_SERVICE_'],
  'tun-interface': ['TUN_INTERFACE_'],
  'physical-interface': ['PHYSICAL_INTERFACE_'],
  'capture-tools': ['CAPTURE_'],
  chrome: ['CHROME_'],
  output: ['OUTPUT_', 'DISK_'],
}

function checksFor(
  report: CompleteEnvironmentReport | undefined,
  id: EnvironmentSummaryItem['id'],
) {
  const prefixes = checkPrefixes[id]
  return (
    report?.checks.filter((check) =>
      prefixes.some((prefix) => check.code.startsWith(prefix)),
    ) ?? []
  )
}

function stateFor(checks: DiagnosticCheck[]): EnvironmentItemState {
  if (checks.some((check) => !check.ok && check.severity === 'error')) {
    return 'error'
  }
  if (checks.some((check) => !check.ok)) return 'warning'
  if (checks.length > 0 && checks.every((check) => check.ok)) return 'ready'
  return 'unknown'
}

function detailString(checks: DiagnosticCheck[], key: string) {
  for (const check of checks) {
    if (
      typeof check.details === 'object' &&
      check.details !== null &&
      key in check.details
    ) {
      const value = (check.details as Record<string, unknown>)[key]
      if (typeof value === 'string' && value.trim()) return value
    }
  }
  return undefined
}

export function remediationTargetFor(
  code: string,
): EnvironmentRemediationTarget {
  if (code.startsWith('CORE_')) return 'core'
  if (code.startsWith('TUN_SERVICE_')) return 'tun-service'
  if (code === 'TUN_DISABLED') return 'tun'
  if (
    code.startsWith('TUN_INTERFACE_') ||
    code.startsWith('PHYSICAL_INTERFACE_')
  ) {
    return 'interfaces'
  }
  if (code.startsWith('CAPTURE_')) return 'capture-tools'
  if (code.startsWith('CHROME_')) return 'chrome'
  if (code.startsWith('OUTPUT_') || code.startsWith('DISK_')) return 'output'
  return 'diagnostics'
}

export interface EnvironmentSummaryText {
  labels: Record<EnvironmentSummaryItem['id'], string>
  localController: string
  notChecked: string
  notSelected: string
  disabled: string
  tunServiceReady: string
  tunServiceUnavailable: string
}

const defaultSummaryText: EnvironmentSummaryText = {
  labels: {
    core: 'Core',
    controller: 'Controller',
    tun: 'TUN',
    'tun-interface': 'TUN interface',
    'physical-interface': 'Physical interface',
    'capture-tools': 'Packet capture',
    chrome: 'Chrome / Chromium',
    output: 'Session output',
  },
  localController: 'Local controller',
  notChecked: 'Not checked',
  notSelected: 'Not selected',
  disabled: 'Disabled',
  tunServiceReady: 'Enabled · service ready',
  tunServiceUnavailable: 'Enabled · service unavailable',
}

export function buildEnvironmentSummary(
  report?: CompleteEnvironmentReport,
  request?: EnvironmentRequest,
  text: EnvironmentSummaryText = defaultSummaryText,
): EnvironmentSummaryItem[] {
  const controllerChecks = checksFor(report, 'controller')
  const captureChecks = checksFor(report, 'capture-tools')
  const chromeChecks = checksFor(report, 'chrome')
  const outputChecks = checksFor(report, 'output')

  return [
    {
      id: 'core',
      label: text.labels.core,
      value: report?.integration.current_core || text.notChecked,
      state:
        report?.integration.current_core === 'verge-mihomo-tt'
          ? 'ready'
          : stateFor(checksFor(report, 'core')),
    },
    {
      id: 'controller',
      label: text.labels.controller,
      value: detailString(controllerChecks, 'endpoint') || text.localController,
      state: stateFor(controllerChecks),
    },
    {
      id: 'tun',
      label: text.labels.tun,
      value: report
        ? report.integration.tun_enabled
          ? report.integration.service_available
            ? text.tunServiceReady
            : text.tunServiceUnavailable
          : text.disabled
        : text.notChecked,
      state: stateFor(checksFor(report, 'tun')),
    },
    {
      id: 'tun-interface',
      label: text.labels['tun-interface'],
      value: request?.tun_interface || text.notSelected,
      state: stateFor(checksFor(report, 'tun-interface')),
    },
    {
      id: 'physical-interface',
      label: text.labels['physical-interface'],
      value: request?.physical_interface || text.notSelected,
      state: stateFor(checksFor(report, 'physical-interface')),
    },
    {
      id: 'capture-tools',
      label: text.labels['capture-tools'],
      value:
        [
          detailString(captureChecks, 'tshark'),
          detailString(captureChecks, 'dumpcap'),
        ]
          .filter(Boolean)
          .join(' · ') || 'tshark / dumpcap',
      state: stateFor(captureChecks),
    },
    {
      id: 'chrome',
      label: text.labels.chrome,
      value:
        detailString(chromeChecks, 'path') ||
        request?.chrome_binary ||
        text.notSelected,
      state: stateFor(chromeChecks),
    },
    {
      id: 'output',
      label: text.labels.output,
      value:
        detailString(outputChecks, 'path') ||
        request?.output_root ||
        text.notSelected,
      state: stateFor(outputChecks),
    },
  ]
}
