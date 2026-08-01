import type { FlowRecord, NormalizedFlowTuple } from '@/types/traffic-tracer'

function addressWithPort(ip: string, port: number) {
  return ip.includes(':') ? `[${ip}]:${port}` : `${ip}:${port}`
}

export function formatFlowTuple(tuple: NormalizedFlowTuple | null) {
  if (!tuple) return 'No complete post-proxy tuple'
  const destination = tuple.dst_host
    ? `${tuple.dst_host} (${addressWithPort(tuple.dst_ip, tuple.dst_port)})`
    : addressWithPort(tuple.dst_ip, tuple.dst_port)
  const value = `${addressWithPort(tuple.src_ip, tuple.src_port)} → ${destination}`
  return tuple.complete ? value : `Incomplete · ${value}`
}

export function flowSearchText(flow: FlowRecord) {
  const tupleValues = [flow.pre_flow, flow.post_flow]
    .filter((tuple): tuple is NormalizedFlowTuple => tuple !== null)
    .flatMap((tuple) => [
      tuple.network,
      tuple.src_ip,
      String(tuple.src_port),
      tuple.dst_ip,
      String(tuple.dst_port),
      tuple.dst_host ?? '',
      tuple.source,
      tuple.scope,
    ])
  return [
    flow.flow_id,
    flow.protocol,
    flow.match.status,
    flow.match.reason,
    flow.url ?? '',
    flow.resource_type ?? '',
    flow.relation ?? '',
    flow.conn_id ?? '',
    flow.outer_conn_id ?? '',
    ...flow.request_ids,
    ...tupleValues,
  ]
    .join(' ')
    .toLocaleLowerCase()
}

export function filterFlows(flows: FlowRecord[], filter: string) {
  const normalized = filter.trim().toLocaleLowerCase()
  if (!normalized) return flows
  return flows.filter((flow) => flowSearchText(flow).includes(normalized))
}
