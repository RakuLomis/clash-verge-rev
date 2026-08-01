import { SearchRounded } from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  MenuItem,
  Paper,
  Stack,
  TextField,
  Typography,
} from '@mui/material'
import { useMutation } from '@tanstack/react-query'
import { useMemo, useState } from 'react'

import { useTrafficTracerSessions } from '@/hooks/use-traffic-tracer-sessions'
import { queryTrafficTracerFlows } from '@/services/cmds'
import type { FlowRecord, SessionManifest } from '@/types/traffic-tracer'

import { TrafficTracerFlowDetail } from './flow-detail'
import {
  defaultFlowQueryDraft,
  flowQueryRequest,
  validateFlowQuery,
  type FlowQueryDraft,
} from './flow-query-form-model'
import { TrafficTracerFlowTable } from './flow-table'

const QUERY_LIMIT = 1000

interface SessionQueryFailure {
  sessionId: string
  domain: string
  message: string
}

interface CrossSessionResult {
  flows: FlowRecord[]
  failures: SessionQueryFailure[]
  queriedSessions: number
}

async function querySessionFlows(
  session: SessionManifest,
  draft: FlowQueryDraft,
) {
  const first = await queryTrafficTracerFlows(
    flowQueryRequest(session.session_id, draft, 0, QUERY_LIMIT),
  )
  const items = [...first.items]
  for (let offset = QUERY_LIMIT; offset < first.total; offset += QUERY_LIMIT) {
    const page = await queryTrafficTracerFlows(
      flowQueryRequest(session.session_id, draft, offset, QUERY_LIMIT),
    )
    items.push(...page.items)
  }
  return items
}

async function queryAllSessions(
  sessions: SessionManifest[],
  draft: FlowQueryDraft,
): Promise<CrossSessionResult> {
  const settled = await Promise.allSettled(
    sessions.map((session) => querySessionFlows(session, draft)),
  )
  const flows: FlowRecord[] = []
  const failures: SessionQueryFailure[] = []
  settled.forEach((result, index) => {
    const session = sessions[index]
    if (result.status === 'fulfilled') {
      flows.push(...result.value)
    } else {
      failures.push({
        sessionId: session.session_id,
        domain: session.target.domain,
        message: String(result.reason),
      })
    }
  })
  return { flows, failures, queriedSessions: sessions.length }
}

export function TrafficTracerFlowQueryForm({
  enabled = true,
}: {
  enabled?: boolean
}) {
  const [draft, setDraft] = useState(defaultFlowQueryDraft)
  const [submitted, setSubmitted] = useState(false)
  const [offset, setOffset] = useState(0)
  const [limit, setLimit] = useState(20)
  const [selectedFlow, setSelectedFlow] = useState<FlowRecord | null>(null)
  const { sessions, sessionsQuery } = useTrafficTracerSessions(
    0,
    Number.MAX_SAFE_INTEGER,
    enabled,
  )
  const errors = useMemo(() => validateFlowQuery(draft), [draft])

  const queryMutation = useMutation({
    mutationFn: () => queryAllSessions(sessions, draft),
    onSuccess: () => setOffset(0),
  })

  const update = <Key extends keyof FlowQueryDraft>(
    key: Key,
    value: FlowQueryDraft[Key],
  ) => {
    queryMutation.reset()
    setSelectedFlow(null)
    setDraft((current) => ({ ...current, [key]: value }))
  }

  const submit = () => {
    setSubmitted(true)
    if (Object.keys(errors).length > 0 || sessions.length === 0) return
    queryMutation.mutate()
  }

  const result = queryMutation.data
  const pageFlows = result?.flows.slice(offset, offset + limit) ?? []
  const selectedSession = selectedFlow
    ? sessions.find((session) => session.session_id === selectedFlow.session_id)
    : undefined

  return (
    <Stack spacing={2} data-testid="traffic-tracer-flow-query">
      <Paper variant="outlined" sx={{ p: 2 }}>
        <Stack spacing={2}>
          <Box>
            <Typography variant="h6" sx={{ fontSize: 17, fontWeight: 600 }}>
              Query pre-proxy five-tuple
            </Typography>
            <Typography variant="body2" color="text.secondary">
              Search every loaded Session for all matching logical Flows and
              their observed post-proxy tuples.
            </Typography>
          </Box>

          <Box
            sx={{
              display: 'grid',
              gridTemplateColumns: {
                xs: '1fr',
                md: '140px minmax(180px, 1fr) 130px minmax(180px, 1fr) 130px',
              },
              gap: 1.5,
            }}
          >
            <TextField
              select
              label="Network"
              value={draft.network}
              onChange={(event) =>
                update(
                  'network',
                  event.target.value as FlowQueryDraft['network'],
                )
              }
            >
              <MenuItem value="tcp">TCP</MenuItem>
              <MenuItem value="udp">UDP</MenuItem>
            </TextField>
            <TextField
              label="Source IP"
              value={draft.src_ip}
              error={submitted && Boolean(errors.src_ip)}
              helperText={submitted ? errors.src_ip : 'IPv4 or IPv6'}
              onChange={(event) => update('src_ip', event.target.value)}
            />
            <TextField
              label="Source port"
              value={draft.src_port}
              error={submitted && Boolean(errors.src_port)}
              helperText={submitted ? errors.src_port : undefined}
              onChange={(event) => update('src_port', event.target.value)}
            />
            <TextField
              label="Destination IP"
              value={draft.dst_ip}
              error={submitted && Boolean(errors.dst_ip)}
              helperText={submitted ? errors.dst_ip : 'IPv4 or IPv6'}
              onChange={(event) => update('dst_ip', event.target.value)}
            />
            <TextField
              label="Destination port"
              value={draft.dst_port}
              error={submitted && Boolean(errors.dst_port)}
              helperText={submitted ? errors.dst_port : undefined}
              onChange={(event) => update('dst_port', event.target.value)}
            />
          </Box>

          <Stack
            direction="row"
            spacing={1}
            sx={{ alignItems: 'center', justifyContent: 'space-between' }}
          >
            <Typography variant="body2" color="text.secondary">
              {enabled
                ? `${sessions.length} Sessions available`
                : 'Check the environment to load Sessions'}
            </Typography>
            <Button
              variant="contained"
              startIcon={<SearchRounded />}
              disabled={
                !enabled ||
                sessionsQuery.isFetching ||
                queryMutation.isPending ||
                sessions.length === 0
              }
              onClick={submit}
            >
              {queryMutation.isPending ? 'Querying…' : 'Query all Sessions'}
            </Button>
          </Stack>
        </Stack>
      </Paper>

      {queryMutation.error && (
        <Alert severity="error">{String(queryMutation.error)}</Alert>
      )}
      {result?.failures.map((failure) => (
        <Alert key={failure.sessionId} severity="warning">
          {failure.domain}: {failure.message}
        </Alert>
      ))}
      {result && (
        <Alert severity={result.flows.length > 0 ? 'success' : 'info'}>
          Found {result.flows.length} logical Flow matches across{' '}
          {result.queriedSessions} Sessions.
        </Alert>
      )}

      {(result || queryMutation.isPending) && (
        <TrafficTracerFlowTable
          flows={pageFlows}
          total={result?.flows.length ?? 0}
          offset={offset}
          limit={limit}
          loading={queryMutation.isPending}
          onPageChange={(nextOffset, nextLimit) => {
            setOffset(nextOffset)
            setLimit(nextLimit)
          }}
          onSelect={setSelectedFlow}
        />
      )}

      <TrafficTracerFlowDetail
        flow={selectedFlow}
        sessionArtifacts={selectedSession?.artifacts}
        onClose={() => setSelectedFlow(null)}
      />
    </Stack>
  )
}
