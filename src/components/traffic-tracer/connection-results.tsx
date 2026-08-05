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
  return `${name}: ${value.matched}/${value.total} matched · ${value.ambiguous} ambiguous · ${value.unmatched} unmatched`
}

function endpoint(flow: ConnectionIndexRecord['pre_flow'] | null) {
  if (!flow) return '—'
  const src = flow.src_ip.includes(':') ? `[${flow.src_ip}]` : flow.src_ip
  const dst = flow.dst_ip.includes(':') ? `[${flow.dst_ip}]` : flow.dst_ip
  return `${flow.network} ${src}:${flow.src_port} → ${dst}:${flow.dst_port}`
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
  return (
    <Stack spacing={2} data-testid="traffic-tracer-connection-results">
      {summary && (
        <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap' }}>
          <Chip
            label={partitionLabel(
              'Browser requests',
              summary.coverage.browser_requests,
            )}
          />
          <Chip
            label={partitionLabel(
              'Transport connections',
              summary.coverage.transport_connections,
            )}
          />
          <Chip
            label={`Core flows: ${summary.coverage.core_logical_flows.with_post_flow}/${summary.coverage.core_logical_flows.total} with post flow · ${summary.coverage.core_logical_flows.shared} shared`}
          />
        </Stack>
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
                    <Typography variant="body2">
                      {connection.request_ids.length} request(s)
                    </Typography>
                  </TableCell>
                  <TableCell>{endpoint(connection.pre_flow)}</TableCell>
                  <TableCell>{endpoint(connection.post_flow)}</TableCell>
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
