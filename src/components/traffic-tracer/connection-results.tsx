import {
  Alert,
  Box,
  Chip,
  CircularProgress,
  Paper,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Typography,
} from '@mui/material'
import { useTranslation } from 'react-i18next'

import type {
  ConnectionIndexRecord,
  CoveragePartition,
  CoverageSummary,
  RequestIndexRecord,
} from '@/types/traffic-tracer'

export interface TrafficTracerConnectionResultsProps {
  summary?: CoverageSummary
  requests: RequestIndexRecord[]
  connections: ConnectionIndexRecord[]
  isLoading?: boolean
  unavailable?: boolean
}

function partitionLabel(name: string, value: CoveragePartition) {
  const nonNetwork = value.non_network
    ? ` · ${value.non_network} non-network`
    : ''
  return `${name}: ${value.matched}/${value.total} matched · ${value.ambiguous} ambiguous · ${value.unmatched} unmatched${nonNetwork}`
}

function byteSize(value: number) {
  if (value >= 1024 ** 3) return `${(value / 1024 ** 3).toFixed(2)} GiB`
  if (value >= 1024 ** 2) return `${(value / 1024 ** 2).toFixed(1)} MiB`
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KiB`
  return `${value} B`
}

function endpoint(flow: ConnectionIndexRecord['pre_flow'] | null) {
  if (!flow) return '—'
  const src = flow.src_ip.includes(':') ? `[${flow.src_ip}]` : flow.src_ip
  const dst = flow.dst_ip.includes(':') ? `[${flow.dst_ip}]` : flow.dst_ip
  return `${flow.network} ${src}:${flow.src_port} → ${dst}:${flow.dst_port}`
}

function egressOutcome(connection: ConnectionIndexRecord) {
  return connection.egress?.outcome || connection.egress?.mode || 'unknown'
}

function noSocketEgress(connection: ConnectionIndexRecord) {
  if (connection.post_flow_disposition) {
    return connection.post_flow_disposition === 'explicit_no_socket'
  }
  return ['rejected', 'rejected_drop', 'internal_dns'].includes(
    egressOutcome(connection),
  )
}

function postFlowAbsenceLabel(connection: ConnectionIndexRecord) {
  switch (connection.post_flow_disposition) {
    case 'explicit_no_socket':
      return 'Not applicable · ' + egressOutcome(connection)
    case 'failed_before_socket':
      return (
        'Failed before socket · ' +
        (connection.terminal?.error_class ||
          connection.terminal?.status ||
          'unknown failure')
      )
    case 'local_not_applicable':
      return 'Local endpoint · post-flow not applicable'
    case 'unexpected_missing':
      return 'Unexpected post-flow absence'
    default:
      return noSocketEgress(connection)
        ? 'Not applicable · ' + egressOutcome(connection)
        : '—'
  }
}

function egressColor(connection: ConnectionIndexRecord) {
  const outcome = egressOutcome(connection)
  if (outcome === 'proxy') return 'primary' as const
  if (outcome === 'direct') return 'success' as const
  if (outcome === 'rejected' || outcome === 'rejected_drop') {
    return 'error' as const
  }
  if (outcome === 'internal_dns' || outcome === 'pass') {
    return 'info' as const
  }
  return 'default' as const
}

function localOnlyConnectionIds(requests: RequestIndexRecord[]) {
  const observations = new Map<string, Set<string>>()
  requests.forEach((request) => {
    if (!request.connection_id) return
    const values = observations.get(request.connection_id) || new Set<string>()
    values.add(request.network_observation || 'unknown')
    observations.set(request.connection_id, values)
  })
  return new Set(
    [...observations.entries()]
      .filter(([, values]) =>
        [...values].every((value) => value === 'local_endpoint'),
      )
      .map(([connectionId]) => connectionId),
  )
}

function groupedConnectionFailures(
  connections: ConnectionIndexRecord[],
  localConnectionIds: Set<string>,
) {
  const groups = new Map<string, number>()
  connections.forEach((connection) => {
    if (localConnectionIds.has(connection.connection_id)) return
    if (
      connection.post_flow_disposition
        ? connection.post_flow_disposition !== 'failed_before_socket'
        : !connection.terminal?.status.endsWith('error')
    )
      return
    let host = 'unknown host'
    if (connection.primary_url) {
      try {
        host = new URL(connection.primary_url).hostname || host
      } catch {
        // Retain the value-safe fallback for malformed legacy URLs.
      }
    }
    const errorClass = connection.terminal?.error_class || 'transport_error'
    const key = `${host} · ${errorClass}`
    groups.set(key, (groups.get(key) || 0) + 1)
  })
  return [...groups.entries()].sort(([left], [right]) =>
    left.localeCompare(right),
  )
}

export function TrafficTracerConnectionResults({
  summary,
  requests,
  connections,
  isLoading = false,
  unavailable = false,
}: TrafficTracerConnectionResultsProps) {
  const { t } = useTranslation()
  if (isLoading) {
    return (
      <Stack sx={{ alignItems: 'center', py: 3 }}>
        <CircularProgress size={24} />
      </Stack>
    )
  }
  if (unavailable) {
    return (
      <Alert severity="info">
        This legacy Session has no connection-centric analysis artifacts. Run
        analysis again to create a v2 generation.
      </Alert>
    )
  }
  const pageCoverage = summary?.coverage.page_attributed
  const globalCoverage = summary?.coverage.capture_global
  const browserCoverage =
    pageCoverage?.browser_requests ?? summary?.coverage.browser_requests
  const transportCoverage =
    pageCoverage?.transport_connections ??
    summary?.coverage.transport_connections
  const quality = summary?.quality
  const pageIntegrityState =
    summary?.analysis_integrity?.page_attributed.state ?? summary?.quality_state
  const globalIntegrityState =
    summary?.analysis_integrity?.capture_global.state ??
    summary?.capture_global_quality_state
  const pageNetworkOutcome = summary?.network_outcome?.page_attributed
  const globalNetworkOutcome = summary?.network_outcome?.capture_global
  const pageQuality = quality?.page_attributed ?? quality
  const qualityWarnings = summary?.warnings ?? []
  const pageWarnings = qualityWarnings.filter(
    (warning) => warning.scope !== 'capture_global',
  )
  const globalWarnings = qualityWarnings.filter(
    (warning) => warning.scope === 'capture_global',
  )
  const localConnectionIds = localOnlyConnectionIds(requests)
  const localEndpointCount =
    pageQuality?.egress_establishment.not_applicable_local_endpoint ??
    localConnectionIds.size
  const noSocketOutcomeCount =
    pageQuality?.egress_establishment.not_applicable_outcome ??
    connections.filter(noSocketEgress).length
  const globalLocalEndpointCount =
    globalCoverage?.core_logical_flows.local_not_applicable ??
    globalCoverage?.core_logical_flows.not_applicable_local_endpoint ??
    quality?.capture_global?.logical_flows.not_applicable_local_endpoint ??
    0
  const globalNoSocketOutcomeCount =
    globalCoverage?.core_logical_flows.explicit_no_socket ??
    globalCoverage?.core_logical_flows.not_applicable_outcome ??
    quality?.capture_global?.logical_flows.not_applicable_outcome ??
    0
  const globalUnexpectedMissingCount = Math.max(
    0,
    globalCoverage?.core_logical_flows.unexpected_missing ??
      (globalCoverage?.core_logical_flows.missing_post_flow ?? 0) -
        globalLocalEndpointCount -
        globalNoSocketOutcomeCount,
  )
  const applicableEgress = Math.max(
    0,
    (pageQuality?.egress_establishment.total ?? 0) -
      localEndpointCount -
      noSocketOutcomeCount,
  )
  const applicablePcap =
    pageQuality?.pcap_extraction.applicable ??
    Math.max(
      0,
      (pageQuality?.pcap_extraction.total ?? 0) -
        (pageQuality?.pcap_extraction.post_not_applicable ?? 0),
    )
  const unavailablePcap = Math.max(
    0,
    applicablePcap - (pageQuality?.pcap_extraction.complete_pairs ?? 0),
  )
  const connectionFailures = groupedConnectionFailures(
    connections,
    localConnectionIds,
  )
  return (
    <Stack spacing={2} data-testid="traffic-tracer-connection-results">
      {localEndpointCount > 0 && (
        <Alert severity="info">
          Local endpoint probes: {localEndpointCount} · pre-proxy evidence
          retained · post-proxy flow not applicable
        </Alert>
      )}
      {noSocketOutcomeCount > 0 && (
        <Alert severity="info">
          Explicit no-socket egress outcomes: {noSocketOutcomeCount} · rejected
          or internal DNS flows retain pre-proxy evidence
        </Alert>
      )}
      {pageIntegrityState && pageIntegrityState !== 'passed' && (
        <Alert severity={pageIntegrityState === 'failed' ? 'error' : 'warning'}>
          <Typography variant="subtitle2">
            Page analysis integrity: {pageIntegrityState}
          </Typography>
          {pageWarnings.length > 0 && (
            <Stack
              direction="row"
              spacing={1}
              sx={{ mt: 0.5, flexWrap: 'wrap' }}
            >
              {pageWarnings.map((warning) => (
                <Chip
                  key={warning.code}
                  size="small"
                  label={`${warning.code}: ${warning.count}`}
                  title={warning.message}
                />
              ))}
            </Stack>
          )}
        </Alert>
      )}
      {globalIntegrityState && globalIntegrityState !== 'passed' && (
        <Alert severity={globalIntegrityState === 'failed' ? 'error' : 'info'}>
          <Typography variant="subtitle2">
            Capture-global analysis integrity: {globalIntegrityState}
          </Typography>
          {globalWarnings.length > 0 && (
            <Stack
              direction="row"
              spacing={1}
              sx={{ mt: 0.5, flexWrap: 'wrap' }}
            >
              {globalWarnings.map((warning) => (
                <Chip
                  key={`${warning.scope}:${warning.code}`}
                  size="small"
                  variant="outlined"
                  label={`${warning.code}: ${warning.count}`}
                  title={warning.message}
                />
              ))}
            </Stack>
          )}
        </Alert>
      )}
      {pageNetworkOutcome &&
        pageNetworkOutcome.state !== 'healthy' &&
        pageNetworkOutcome.state !== 'not_applicable' && (
          <Alert
            severity={
              pageNetworkOutcome.state === 'failed' ? 'error' : 'warning'
            }
          >
            Page network outcome: {pageNetworkOutcome.state} ·{' '}
            {pageNetworkOutcome.established}
            established · {pageNetworkOutcome.failed_before_socket} failed
            before socket
          </Alert>
        )}
      {globalNetworkOutcome &&
        globalNetworkOutcome.failed_before_socket > 0 && (
          <Alert severity="info">
            Capture background network failures:{' '}
            {globalNetworkOutcome.failed_before_socket} · analysis evidence
            remains complete
          </Alert>
        )}
      {summary && (
        <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap' }}>
          {summary.playback && (
            <Chip
              variant="outlined"
              color={summary.playback.primary_goal_met ? 'success' : 'warning'}
              label={`Playback: ${summary.playback.primary_content_seconds.toFixed(1)}/${summary.playback.desired_primary_seconds}s primary · fixed ${summary.playback.observation_window_seconds}s · ${summary.playback.quality}${summary.playback.skip_attempts ? ` · ${summary.playback.skip_attempts} skip click` : ''}`}
            />
          )}
          {summary.trace_snapshot && (
            <Chip
              variant="outlined"
              color={
                summary.trace_snapshot.source === 'mihomo_barrier'
                  ? 'success'
                  : 'warning'
              }
              label={`Trace snapshot: ${summary.trace_snapshot.source === 'mihomo_barrier' ? 'barrier' : summary.trace_snapshot.source} · ${summary.trace_snapshot.trace_count} trace${summary.trace_snapshot.trace_count === 1 ? '' : 's'} · ${summary.trace_snapshot.late_event_count} late events excluded`}
            />
          )}
          {summary.storage && (
            <Chip
              variant="outlined"
              label={`Storage: ${byteSize(summary.storage.capture_bytes)} capture · ${byteSize(summary.storage.raw_packet_capture_bytes)} raw PCAP · compression ${summary.storage.compression}`}
            />
          )}
          <Chip label={partitionLabel('Browser requests', browserCoverage!)} />
          <Chip
            label={partitionLabel('Transport connections', transportCoverage!)}
          />
          <Chip
            label={
              pageCoverage
                ? `Page flows: ${pageCoverage.logical_flows.with_post_flow} socket established · ${pageCoverage.logical_flows.explicit_no_socket ?? pageCoverage.logical_flows.not_applicable_outcome ?? 0} explicit no-socket · ${pageCoverage.logical_flows.unexpected_missing ?? pageCoverage.logical_flows.missing_post_flow} unexpected missing · ${pageCoverage.logical_flows.shared} shared`
                : `Core flows: ${summary.coverage.core_logical_flows.with_post_flow} socket established · ${summary.coverage.core_logical_flows.explicit_no_socket ?? summary.coverage.core_logical_flows.not_applicable_outcome ?? 0} explicit no-socket · ${summary.coverage.core_logical_flows.unexpected_missing ?? summary.coverage.core_logical_flows.missing_post_flow} unexpected missing · ${summary.coverage.core_logical_flows.shared} shared`
            }
          />
          {globalCoverage && (
            <Chip
              variant="outlined"
              label={`Capture-global flows: ${globalCoverage.core_logical_flows.with_post_flow} socket established · ${globalNoSocketOutcomeCount} explicit no-socket · ${globalLocalEndpointCount} local N/A · ${globalUnexpectedMissingCount} unexpected missing`}
            />
          )}
          {pageQuality && (
            <Chip
              variant="outlined"
              label={
                noSocketOutcomeCount > 0
                  ? `Egress: ${pageQuality.egress_establishment.established}/${applicableEgress} socket-applicable established · ${pageQuality.egress_establishment.failed_before_socket} dial failed · ${noSocketOutcomeCount} explicit no-socket`
                  : localEndpointCount > 0
                    ? `Egress: ${pageQuality.egress_establishment.established}/${applicableEgress} applicable established · ${pageQuality.egress_establishment.failed_before_socket} dial failed · ${localEndpointCount} local N/A`
                    : `Egress: ${pageQuality.egress_establishment.established}/${pageQuality.egress_establishment.total} established · ${pageQuality.egress_establishment.failed_before_socket} dial failed`
              }
            />
          )}
          {pageQuality?.pcap_extraction.requested === false && (
            <Chip
              variant="outlined"
              color="info"
              label={t(
                'settings.trafficTracer.sessions.packetVerificationNotRequested',
              )}
            />
          )}
          {pageQuality?.pcap_extraction.requested && (
            <Chip
              variant="outlined"
              label={
                localEndpointCount > 0
                  ? `PCAP pairs: ${pageQuality.pcap_extraction.complete_pairs}/${applicablePcap} applicable complete · pre ${pageQuality.pcap_extraction.pre_success} · post ${pageQuality.pcap_extraction.post_success} · ${unavailablePcap} unavailable · ${pageQuality.pcap_extraction.post_not_applicable ?? localEndpointCount} local post N/A`
                  : `PCAP pairs: ${pageQuality.pcap_extraction.complete_pairs}/${applicablePcap} applicable complete · pre ${pageQuality.pcap_extraction.pre_success} · post ${pageQuality.pcap_extraction.post_success} · ${unavailablePcap} unavailable`
              }
            />
          )}
        </Stack>
      )}

      {connectionFailures.length > 0 && (
        <Alert severity="warning">
          <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
            Page connection failures
          </Typography>
          <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap' }}>
            {connectionFailures.map(([label, count]) => (
              <Chip key={label} size="small" label={`${label}: ${count}`} />
            ))}
          </Stack>
        </Alert>
      )}

      <Box>
        <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
          Browser requests ({requests.length})
        </Typography>
        <TableContainer component={Paper} variant="outlined">
          <Table size="small">
            <TableHead>
              <TableRow>
                <TableCell>Request / URL</TableCell>
                <TableCell>Connection</TableCell>
                <TableCell>Attribution</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {requests.map((request) => (
                <TableRow key={request.request_id}>
                  <TableCell>
                    <Typography variant="caption">
                      {request.request_id}
                    </Typography>
                    <Typography variant="body2" sx={{ wordBreak: 'break-all' }}>
                      {request.url}
                    </Typography>
                  </TableCell>
                  <TableCell>
                    {request.connection_id ||
                      request.candidate_connection_ids.join(', ') ||
                      '—'}
                  </TableCell>
                  <TableCell>
                    {request.network_observation &&
                    !['network', 'unknown'].includes(
                      request.network_observation,
                    ) ? (
                      <Chip
                        size="small"
                        color="info"
                        label={`non-network · ${request.network_observation}`}
                      />
                    ) : (
                      <Chip
                        size="small"
                        color={
                          request.attribution.status === 'matched'
                            ? 'success'
                            : request.attribution.status === 'ambiguous'
                              ? 'warning'
                              : 'default'
                        }
                        label={`${request.attribution.status} · ${request.attribution.method}`}
                      />
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </TableContainer>
      </Box>

      <Box>
        <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
          Unique connections ({connections.length})
        </Typography>
        <TableContainer component={Paper} variant="outlined">
          <Table size="small">
            <TableHead>
              <TableRow>
                <TableCell>Connection / requests</TableCell>
                <TableCell>Pre-proxy flow</TableCell>
                <TableCell>Post-proxy flow</TableCell>
                <TableCell>Egress / sharing</TableCell>
                <TableCell>Lifecycle</TableCell>
                <TableCell>Match</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {connections.map((connection) => (
                <TableRow key={connection.connection_id}>
                  <TableCell>
                    <Typography variant="caption">
                      {connection.connection_id}
                    </Typography>
                    <Typography
                      variant="body2"
                      sx={{ maxWidth: 360, overflowWrap: 'anywhere' }}
                    >
                      {connection.primary_url || 'No attributed URL'}
                    </Typography>
                    <Typography variant="body2">
                      {connection.request_ids.length} request(s) ·{' '}
                      {connection.urls.length} URL(s)
                    </Typography>
                    {connection.attribution_scope && (
                      <Typography variant="caption" sx={{ display: 'block' }}>
                        Scope: {connection.attribution_scope} · evidence:{' '}
                        {connection.attribution_evidence?.join(', ') || '—'}
                      </Typography>
                    )}
                  </TableCell>
                  <TableCell>
                    {endpoint(connection.pre_flow)}
                    <Typography variant="caption" sx={{ display: 'block' }}>
                      app: {connection.application_protocol || 'unknown'}
                      {connection.attempted_protocols?.length
                        ? ` · attempted ${connection.attempted_protocols.join(', ')}`
                        : ''}
                    </Typography>
                  </TableCell>
                  <TableCell>
                    {connection.post_flow
                      ? endpoint(connection.post_flow)
                      : postFlowAbsenceLabel(connection)}
                  </TableCell>
                  <TableCell>
                    {connection.egress ? (
                      <>
                        <Chip
                          size="small"
                          color={egressColor(connection)}
                          label={egressOutcome(connection)}
                        />
                        <Typography variant="caption" sx={{ display: 'block' }}>
                          {connection.egress.selection_chain.join(' → ') || '—'}
                        </Typography>
                      </>
                    ) : (
                      '—'
                    )}
                    {connection.sharing && (
                      <Typography variant="caption" sx={{ display: 'block' }}>
                        {[
                          connection.sharing.request_multiplexed &&
                            'request multiplexing',
                          connection.sharing.post_flow_shared &&
                            'shared post-flow',
                          connection.sharing.outer_connection_reused &&
                            'outer reuse',
                        ]
                          .filter(Boolean)
                          .join(' · ') || 'not shared'}
                      </Typography>
                    )}
                  </TableCell>
                  <TableCell>
                    {connection.terminal ? (
                      <>
                        <Chip
                          size="small"
                          color={
                            connection.terminal.status === 'closed'
                              ? 'success'
                              : connection.terminal.status.endsWith('error')
                                ? 'error'
                                : 'warning'
                          }
                          label={`${connection.terminal.status}${
                            connection.terminal.stage
                              ? ` · ${connection.terminal.stage}`
                              : ''
                          }`}
                        />
                        {connection.terminal.error && (
                          <Typography
                            variant="caption"
                            title={connection.terminal.error}
                            sx={{
                              display: 'block',
                              maxWidth: 280,
                              overflow: 'hidden',
                              textOverflow: 'ellipsis',
                              whiteSpace: 'nowrap',
                            }}
                          >
                            {connection.terminal.error}
                          </Typography>
                        )}
                        {connection.terminal.error_class && (
                          <Typography
                            variant="caption"
                            sx={{ display: 'block' }}
                          >
                            {connection.terminal.error_class} ·{' '}
                            {connection.terminal.error_class_source || 'legacy'}
                          </Typography>
                        )}
                      </>
                    ) : (
                      '—'
                    )}
                  </TableCell>
                  <TableCell>
                    <Chip
                      size="small"
                      color={
                        connection.match.status === 'matched'
                          ? 'success'
                          : connection.match.status === 'ambiguous'
                            ? 'warning'
                            : 'default'
                      }
                      label={`${connection.match.status} · ${connection.match.method}`}
                    />
                    {connection.match.time_evidence?.available && (
                      <Typography variant="caption" sx={{ display: 'block' }}>
                        time Δ {connection.match.time_evidence.delta_ms} ms ·{' '}
                        {connection.match.time_evidence.source}
                      </Typography>
                    )}
                    {connection.match.status === 'ambiguous' && (
                      <Typography variant="caption" sx={{ display: 'block' }}>
                        {connection.match.candidates_truncated
                          ? `${connection.match.candidates.length} shown · ${connection.match.candidate_count ?? connection.match.candidates.length} total candidates`
                          : `${connection.match.candidates.length} candidates`}
                      </Typography>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </TableContainer>
      </Box>
    </Stack>
  )
}
