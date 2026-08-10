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

function endpoint(flow: ConnectionIndexRecord['pre_flow'] | null) {
  if (!flow) return '—'
  const src = flow.src_ip.includes(':') ? `[${flow.src_ip}]` : flow.src_ip
  const dst = flow.dst_ip.includes(':') ? `[${flow.dst_ip}]` : flow.dst_ip
  return `${flow.network} ${src}:${flow.src_port} → ${dst}:${flow.dst_port}`
}

function groupedConnectionFailures(connections: ConnectionIndexRecord[]) {
  const groups = new Map<string, number>()
  connections.forEach((connection) => {
    if (!connection.terminal?.status.endsWith('error')) return
    let host = 'unknown host'
    if (connection.primary_url) {
      try {
        host = new URL(connection.primary_url).hostname || host
      } catch {
        // Retain the value-safe fallback for malformed legacy URLs.
      }
    }
    const errorClass = connection.terminal.error_class || 'transport_error'
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
  const pageQuality = quality?.page_attributed ?? quality
  const qualityWarnings = summary?.warnings ?? []
  const pageWarnings = qualityWarnings.filter(
    (warning) => warning.scope !== 'capture_global',
  )
  const globalWarnings = qualityWarnings.filter(
    (warning) => warning.scope === 'capture_global',
  )
  const connectionFailures = groupedConnectionFailures(connections)
  return (
    <Stack spacing={2} data-testid="traffic-tracer-connection-results">
      {summary?.quality_state && summary.quality_state !== 'passed' && (
        <Alert
          severity={summary.quality_state === 'failed' ? 'error' : 'warning'}
        >
          <Typography variant="subtitle2">
            Page analysis quality: {summary.quality_state}
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
      {summary?.capture_global_quality_state &&
        summary.capture_global_quality_state !== 'passed' && (
          <Alert
            severity={
              summary.capture_global_quality_state === 'failed'
                ? 'error'
                : 'info'
            }
          >
            <Typography variant="subtitle2">
              Capture-global diagnostics: {summary.capture_global_quality_state}
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
      {summary && (
        <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap' }}>
          <Chip label={partitionLabel('Browser requests', browserCoverage!)} />
          <Chip
            label={partitionLabel('Transport connections', transportCoverage!)}
          />
          <Chip
            label={
              pageCoverage
                ? `Page flows: ${pageCoverage.logical_flows.with_post_flow}/${pageCoverage.logical_flows.total} with post flow · ${pageCoverage.logical_flows.shared} shared`
                : `Core flows: ${summary.coverage.core_logical_flows.with_post_flow}/${summary.coverage.core_logical_flows.total} with post flow · ${summary.coverage.core_logical_flows.shared} shared`
            }
          />
          {globalCoverage && (
            <Chip
              variant="outlined"
              label={`Capture-global core flows: ${globalCoverage.core_logical_flows.with_post_flow}/${globalCoverage.core_logical_flows.total} with post flow · ${globalCoverage.core_logical_flows.missing_post_flow} missing`}
            />
          )}
          {pageQuality && (
            <Chip
              variant="outlined"
              label={`Egress: ${pageQuality.egress_establishment.established}/${pageQuality.egress_establishment.total} established · ${pageQuality.egress_establishment.failed_before_socket} dial failed`}
            />
          )}
          {pageQuality?.pcap_extraction.requested && (
            <Chip
              variant="outlined"
              label={`PCAP pairs: ${pageQuality.pcap_extraction.complete_pairs}/${pageQuality.pcap_extraction.total} · pre ${pageQuality.pcap_extraction.pre_success} · post ${pageQuality.pcap_extraction.post_success}`}
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
                  <TableCell>{endpoint(connection.post_flow)}</TableCell>
                  <TableCell>
                    {connection.egress ? (
                      <>
                        <Chip
                          size="small"
                          color={
                            connection.egress.mode === 'proxy'
                              ? 'primary'
                              : connection.egress.mode === 'direct'
                                ? 'success'
                                : 'default'
                          }
                          label={connection.egress.mode}
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
                    {connection.match.status === 'ambiguous' && (
                      <Typography variant="caption" sx={{ display: 'block' }}>
                        {connection.match.candidates.length} candidates
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
