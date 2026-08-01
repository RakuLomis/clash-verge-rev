import type { FlowNetwork, FlowQueryRequest } from '@/types/traffic-tracer'

export interface FlowQueryDraft {
  network: FlowNetwork
  src_ip: string
  src_port: string
  dst_ip: string
  dst_port: string
}

export type FlowQueryErrors = Partial<Record<keyof FlowQueryDraft, string>>

export const defaultFlowQueryDraft: FlowQueryDraft = {
  network: 'tcp',
  src_ip: '',
  src_port: '',
  dst_ip: '',
  dst_port: '',
}

function validIpv4(value: string) {
  const parts = value.split('.')
  return (
    parts.length === 4 &&
    parts.every(
      (part) =>
        /^\d{1,3}$/.test(part) &&
        Number(part) >= 0 &&
        Number(part) <= 255 &&
        String(Number(part)) === part,
    ) &&
    value !== '0.0.0.0'
  )
}

function validIpv6(value: string) {
  if (!value.includes(':')) return false
  try {
    const parsed = new URL(`http://[${value}]/`)
    const normalized = parsed.hostname.replace(/^\[|\]$/g, '')
    return !/^(?:(?:0*:){7}0*|::)$/.test(normalized)
  } catch {
    return false
  }
}

export function validFlowIp(value: string) {
  const normalized = value.trim()
  return validIpv4(normalized) || validIpv6(normalized)
}

function validPort(value: string) {
  const port = Number(value)
  return (
    /^\d+$/.test(value) && Number.isInteger(port) && port >= 1 && port <= 65535
  )
}

export function validateFlowQuery(draft: FlowQueryDraft): FlowQueryErrors {
  const errors: FlowQueryErrors = {}
  if (!validFlowIp(draft.src_ip))
    errors.src_ip = 'Enter a valid, non-unspecified IP address.'
  if (!validPort(draft.src_port))
    errors.src_port = 'Port must be between 1 and 65535.'
  if (!validFlowIp(draft.dst_ip))
    errors.dst_ip = 'Enter a valid, non-unspecified IP address.'
  if (!validPort(draft.dst_port))
    errors.dst_port = 'Port must be between 1 and 65535.'
  return errors
}

export function flowQueryRequest(
  sessionId: string,
  draft: FlowQueryDraft,
  offset = 0,
  limit = 1000,
): FlowQueryRequest {
  return {
    session_id: sessionId,
    network: draft.network,
    src_ip: draft.src_ip.trim(),
    src_port: Number(draft.src_port),
    dst_ip: draft.dst_ip.trim(),
    dst_port: Number(draft.dst_port),
    offset,
    limit,
  }
}
