import type {
  CaptureNetwork,
  CaptureOptions,
  CaptureStartRequest,
  BatchStartRequest,
  EnvironmentRequest,
  TargetConfigEntry,
  TargetConfigPreview,
} from '@/types/traffic-tracer'

export type TargetInputMode = 'manual' | 'config'

export interface CaptureFormDraft {
  target_mode: TargetInputMode
  config_path: string
  config_sha256: string
  selected_target_index: number | null
  url: string
  domain: string
  duration_seconds: number
  network: CaptureNetwork
  tun_interface: string
  physical_interface: string
  output_root: string
  chrome_binary: string
  wait_load_timeout: number
  run_label: string
  options: CaptureOptions
  page_type: string
}

export function selectedTargetsInConfigOrder(
  preview: TargetConfigPreview,
  selectedIndexes: ReadonlySet<number>,
) {
  return preview.targets.filter((target) => selectedIndexes.has(target.index))
}

export function batchRequestFromDraft(
  draft: CaptureFormDraft,
  preview: TargetConfigPreview,
  selectedIndexes: ReadonlySet<number>,
): BatchStartRequest {
  const targets = selectedTargetsInConfigOrder(preview, selectedIndexes)
  if (!targets.length) throw new Error('Select at least one batch target.')
  return {
    config_path: preview.config_path,
    config_sha256: preview.sha256,
    targets,
    tun_interface: draft.tun_interface.trim(),
    physical_interface: draft.physical_interface.trim(),
    output_root: draft.output_root.trim(),
    chrome_binary: draft.chrome_binary.trim(),
    options: { ...draft.options, analyze_after_capture: true },
    fail_fast: true,
  }
}

export type CaptureFormErrors = Partial<
  Record<
    | 'url'
    | 'config_path'
    | 'domain'
    | 'duration_seconds'
    | 'tun_interface'
    | 'physical_interface'
    | 'output_root'
    | 'chrome_binary',
    | 'url'
    | 'configFile'
    | 'domain'
    | 'duration'
    | 'tunInterface'
    | 'physicalInterface'
    | 'output'
    | 'chrome'
  >
>

export const defaultCaptureFormDraft: CaptureFormDraft = {
  target_mode: 'manual',
  config_path: '',
  config_sha256: '',
  selected_target_index: null,
  url: '',
  domain: '',
  duration_seconds: 30,
  network: 'all',
  tun_interface: '',
  physical_interface: '',
  output_root: '',
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
}

export function deriveDomain(url: string) {
  try {
    const parsed = new URL(url)
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return ''
    return parsed.hostname.replace(/\.$/, '')
  } catch {
    return ''
  }
}

export function isAbsolutePlatformPath(path: string) {
  const value = path.trim()
  return (
    value.startsWith('/') ||
    /^[A-Za-z]:[\\/]/.test(value) ||
    /^\\\\[^\\]+\\[^\\]+/.test(value)
  )
}

function validDomain(domain: string) {
  return (
    domain.length > 0 &&
    domain.length <= 253 &&
    domain
      .split('.')
      .every(
        (label) =>
          label.length > 0 &&
          label.length <= 63 &&
          !label.startsWith('-') &&
          !label.endsWith('-') &&
          /^[A-Za-z0-9-]+$/.test(label),
      )
  )
}

export function validateCaptureForm(
  draft: CaptureFormDraft,
): CaptureFormErrors {
  const errors: CaptureFormErrors = {}
  if (
    draft.target_mode === 'config' &&
    (!isAbsolutePlatformPath(draft.config_path) ||
      !/^[a-f0-9]{64}$/.test(draft.config_sha256) ||
      draft.selected_target_index === null ||
      draft.selected_target_index < 0)
  ) {
    errors.config_path = 'configFile'
  }
  if (!deriveDomain(draft.url) || /\s/.test(draft.url)) {
    errors.url = 'url'
  }
  if (!validDomain(draft.domain.trim())) {
    errors.domain = 'domain'
  }
  if (
    !Number.isInteger(draft.duration_seconds) ||
    draft.duration_seconds < 1 ||
    draft.duration_seconds > 86_400
  ) {
    errors.duration_seconds = 'duration'
  }
  if (
    !Number.isInteger(draft.wait_load_timeout) ||
    draft.wait_load_timeout < 1 ||
    draft.wait_load_timeout > 3_600 ||
    !/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(draft.run_label)
  ) {
    errors.config_path = 'configFile'
  }
  if (!draft.tun_interface.trim()) {
    errors.tun_interface = 'tunInterface'
  }
  if (!draft.physical_interface.trim()) {
    errors.physical_interface = 'physicalInterface'
  }
  if (!isAbsolutePlatformPath(draft.output_root)) {
    errors.output_root = 'output'
  }
  if (!isAbsolutePlatformPath(draft.chrome_binary)) {
    errors.chrome_binary = 'chrome'
  }
  return errors
}

export function environmentRequestFromDraft(
  draft: CaptureFormDraft,
): EnvironmentRequest {
  return {
    tun_interface: draft.tun_interface.trim(),
    physical_interface: draft.physical_interface.trim(),
    chrome_binary: draft.chrome_binary.trim(),
    output_root: draft.output_root.trim(),
  }
}

export function captureRequestFromDraft(
  draft: CaptureFormDraft,
): CaptureStartRequest {
  return {
    url: draft.url.trim(),
    domain: draft.domain.trim(),
    duration_seconds: draft.duration_seconds,
    network: draft.network,
    tun_interface: draft.tun_interface.trim(),
    physical_interface: draft.physical_interface.trim(),
    output_root: draft.output_root.trim(),
    chrome_binary: draft.chrome_binary.trim(),
    wait_load_timeout: draft.wait_load_timeout,
    run_label: draft.run_label,
    page_type: draft.page_type,
    target_source:
      draft.target_mode === 'config'
        ? {
            mode: 'config',
            config_path: draft.config_path,
            config_sha256: draft.config_sha256,
            target_index: draft.selected_target_index ?? -1,
          }
        : { mode: 'manual' },
    options: { ...draft.options },
  }
}

export function applyTargetConfigEntry(
  draft: CaptureFormDraft,
  preview: TargetConfigPreview,
  target: TargetConfigEntry,
): CaptureFormDraft {
  return {
    ...draft,
    target_mode: 'config',
    config_path: preview.config_path,
    config_sha256: preview.sha256,
    selected_target_index: target.index,
    url: target.url,
    domain: target.domain,
    duration_seconds: target.duration_seconds,
    network: target.network,
    wait_load_timeout: target.wait_load_timeout,
    run_label: target.run_label,
    page_type: target.page_type,
  }
}

export function environmentRequestsEqual(
  left: EnvironmentRequest | null | undefined,
  right: EnvironmentRequest,
) {
  return (
    left?.tun_interface === right.tun_interface &&
    left.physical_interface === right.physical_interface &&
    left.chrome_binary === right.chrome_binary &&
    left.output_root === right.output_root &&
    (left.min_free_bytes ?? null) === (right.min_free_bytes ?? null)
  )
}

export function suggestCaptureInterfaces(interfaces: string[]) {
  const usable = interfaces.filter(Boolean)
  const tunCandidates = usable.filter((name) =>
    /^(meta|mihomo|clash|utun|tun)/i.test(name),
  )
  const tun = tunCandidates.length === 1 ? tunCandidates[0] : ''
  const physical =
    usable.find(
      (name) =>
        name !== tun &&
        !/^(lo|loopback|meta|mihomo|clash|utun|tun|docker|veth|br-)/i.test(
          name,
        ),
    ) ?? ''
  return { tun, physical }
}
