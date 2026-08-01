import type {
  CaptureNetwork,
  CaptureOptions,
  CaptureStartRequest,
  EnvironmentRequest,
} from '@/types/traffic-tracer'

export interface CaptureFormDraft {
  url: string
  domain: string
  duration_seconds: number
  network: CaptureNetwork
  tun_interface: string
  physical_interface: string
  output_root: string
  chrome_binary: string
  options: CaptureOptions
}

export type CaptureFormErrors = Partial<
  Record<
    | 'url'
    | 'domain'
    | 'duration_seconds'
    | 'tun_interface'
    | 'physical_interface'
    | 'output_root'
    | 'chrome_binary',
    string
  >
>

export const defaultCaptureFormDraft: CaptureFormDraft = {
  url: '',
  domain: '',
  duration_seconds: 30,
  network: 'all',
  tun_interface: '',
  physical_interface: '',
  output_root: '',
  chrome_binary: 'google-chrome',
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
  if (!deriveDomain(draft.url) || /\s/.test(draft.url)) {
    errors.url = 'Enter an absolute HTTP(S) URL.'
  }
  if (!validDomain(draft.domain.trim())) {
    errors.domain = 'Enter a valid DNS domain.'
  }
  if (
    !Number.isInteger(draft.duration_seconds) ||
    draft.duration_seconds < 1 ||
    draft.duration_seconds > 86_400
  ) {
    errors.duration_seconds = 'Duration must be between 1 and 86400 seconds.'
  }
  if (!draft.tun_interface.trim()) {
    errors.tun_interface = 'Select the TUN interface.'
  }
  if (!draft.physical_interface.trim()) {
    errors.physical_interface = 'Select the physical interface.'
  }
  if (!isAbsolutePlatformPath(draft.output_root)) {
    errors.output_root = 'Choose an absolute Session output directory.'
  }
  if (!isAbsolutePlatformPath(draft.chrome_binary)) {
    errors.chrome_binary = 'Choose an absolute Chrome/Chromium executable.'
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
    options: { ...draft.options },
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
  const tun =
    usable.find((name) => /^(meta|mihomo|clash|utun|tun)/i.test(name)) ?? ''
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
