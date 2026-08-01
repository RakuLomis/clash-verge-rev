import { SearchRounded, WarningAmberRounded } from '@mui/icons-material'
import {
  Alert,
  Box,
  Chip,
  CircularProgress,
  InputAdornment,
  Paper,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TablePagination,
  TableRow,
  TextField,
  Tooltip,
  Typography,
} from '@mui/material'
import { useMemo, useState } from 'react'

import type { FlowMatchStatus, FlowRecord } from '@/types/traffic-tracer'

import { filterFlows, formatFlowTuple } from './flow-table-model'

export interface TrafficTracerFlowTableProps {
  flows: FlowRecord[]
  total?: number
  offset?: number
  limit?: number
  loading?: boolean
  error?: unknown
  onPageChange?: (offset: number, limit: number) => void
  onSelect?: (flow: FlowRecord) => void
}

const emptyFlows: FlowRecord[] = []

const matchColor: Record<
  FlowMatchStatus,
  'success' | 'warning' | 'default' | 'info'
> = {
  matched: 'success',
  ambiguous: 'warning',
  unmatched: 'default',
  legacy: 'info',
}

function confidenceLabel(confidence: number) {
  if (!Number.isFinite(confidence)) return 'unknown'
  return `${Math.round(Math.min(1, Math.max(0, confidence)) * 100)}%`
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

export function TrafficTracerFlowTable({
  flows = emptyFlows,
  total = flows.length,
  offset = 0,
  limit = 20,
  loading = false,
  error,
  onPageChange,
  onSelect,
}: TrafficTracerFlowTableProps) {
  const [filter, setFilter] = useState('')
  const visibleFlows = useMemo(
    () => filterFlows(flows, filter),
    [filter, flows],
  )
  const page = Math.floor(Math.max(0, offset) / Math.max(1, limit))

  return (
    <Paper variant="outlined" data-testid="traffic-tracer-flow-table">
      <Stack
        direction="row"
        spacing={2}
        sx={{
          alignItems: 'center',
          justifyContent: 'space-between',
          px: 2,
          py: 1.5,
        }}
      >
        <Box>
          <Typography variant="h6" sx={{ fontSize: 17, fontWeight: 600 }}>
            Normalized Flows
          </Typography>
          <Typography variant="body2" color="text.secondary">
            Pre-proxy tuples and their observed post-proxy mappings
          </Typography>
        </Box>
        <TextField
          size="small"
          value={filter}
          placeholder="Filter current results"
          onChange={(event) => setFilter(event.target.value)}
          slotProps={{
            input: {
              startAdornment: (
                <InputAdornment position="start">
                  <SearchRounded fontSize="small" />
                </InputAdornment>
              ),
            },
          }}
        />
      </Stack>

      {error != null && <Alert severity="error">{errorMessage(error)}</Alert>}

      <TableContainer sx={{ position: 'relative', minHeight: 160 }}>
        {loading && (
          <Stack
            sx={{
              position: 'absolute',
              inset: 0,
              zIndex: 1,
              alignItems: 'center',
              justifyContent: 'center',
              bgcolor: 'background.paper',
              opacity: 0.8,
            }}
          >
            <CircularProgress size={28} />
          </Stack>
        )}
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Network</TableCell>
              <TableCell>Session</TableCell>
              <TableCell>Pre-proxy</TableCell>
              <TableCell>Post-proxy</TableCell>
              <TableCell>Match</TableCell>
              <TableCell>Shared</TableCell>
              <TableCell>URL</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {!loading && visibleFlows.length === 0 ? (
              <TableRow>
                <TableCell colSpan={7} align="center" sx={{ py: 5 }}>
                  <Typography color="text.secondary">
                    {filter
                      ? 'No Flows match this filter.'
                      : 'No Flow results.'}
                  </Typography>
                </TableCell>
              </TableRow>
            ) : (
              visibleFlows.map((flow) => (
                <TableRow
                  hover={Boolean(onSelect)}
                  key={`${flow.session_id}:${flow.flow_id}`}
                  data-testid={`flow-row-${flow.session_id}-${flow.flow_id}`}
                  onClick={() => onSelect?.(flow)}
                  sx={{ cursor: onSelect ? 'pointer' : 'default' }}
                >
                  <TableCell>
                    <Chip
                      size="small"
                      variant="outlined"
                      label={flow.protocol.toUpperCase()}
                    />
                  </TableCell>
                  <TableCell sx={{ maxWidth: 150 }}>
                    <Typography
                      variant="caption"
                      title={flow.session_id}
                      sx={{
                        display: 'block',
                        fontFamily: 'monospace',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                      }}
                    >
                      {flow.session_id}
                    </Typography>
                  </TableCell>
                  <TableCell sx={{ minWidth: 260 }}>
                    <Tooltip title={formatFlowTuple(flow.pre_flow)}>
                      <Typography
                        variant="body2"
                        sx={{ fontFamily: 'monospace' }}
                      >
                        {formatFlowTuple(flow.pre_flow)}
                      </Typography>
                    </Tooltip>
                  </TableCell>
                  <TableCell sx={{ minWidth: 260 }}>
                    <Tooltip title={formatFlowTuple(flow.post_flow)}>
                      <Typography
                        variant="body2"
                        color={
                          flow.post_flow?.complete
                            ? 'text.primary'
                            : 'text.secondary'
                        }
                        sx={{ fontFamily: 'monospace' }}
                      >
                        {formatFlowTuple(flow.post_flow)}
                      </Typography>
                    </Tooltip>
                  </TableCell>
                  <TableCell>
                    <Tooltip title={flow.match.reason}>
                      <Chip
                        size="small"
                        color={matchColor[flow.match.status]}
                        label={`${flow.match.status} · ${confidenceLabel(flow.match.confidence)}`}
                      />
                    </Tooltip>
                  </TableCell>
                  <TableCell>
                    {flow.shared ||
                    flow.pre_flow.shared ||
                    flow.post_flow?.shared ? (
                      <Tooltip title="This outer Flow is shared by multiple logical Flows.">
                        <Chip
                          size="small"
                          color="warning"
                          icon={<WarningAmberRounded />}
                          label="Shared"
                        />
                      </Tooltip>
                    ) : (
                      <Typography variant="body2" color="text.secondary">
                        Exclusive
                      </Typography>
                    )}
                  </TableCell>
                  <TableCell sx={{ minWidth: 220, maxWidth: 360 }}>
                    <Typography
                      variant="body2"
                      title={flow.url ?? ''}
                      sx={{
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {flow.url || '—'}
                    </Typography>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </TableContainer>

      <TablePagination
        component="div"
        count={Math.max(0, total)}
        page={page}
        rowsPerPage={Math.max(1, limit)}
        rowsPerPageOptions={[10, 20, 50, 100]}
        onPageChange={(_, nextPage) =>
          onPageChange?.(nextPage * Math.max(1, limit), Math.max(1, limit))
        }
        onRowsPerPageChange={(event) => {
          const nextLimit = Number(event.target.value)
          onPageChange?.(0, nextLimit)
        }}
        sx={{ borderTop: 1, borderColor: 'divider' }}
      />
    </Paper>
  )
}
