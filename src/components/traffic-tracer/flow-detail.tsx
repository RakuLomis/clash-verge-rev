import {
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  Paper,
  Stack,
  Typography,
} from '@mui/material'

import type {
  FlowRecord,
  NormalizedFlowTuple,
  SessionArtifact,
} from '@/types/traffic-tracer'

import { TrafficTracerArtifactList } from './artifact-list'

const emptyArtifacts: SessionArtifact[] = []

export interface TrafficTracerFlowDetailProps {
  flow: FlowRecord | null
  sessionArtifacts?: SessionArtifact[]
  onClose: () => void
}

function endpoint(ip: string, port: number) {
  return ip.includes(':') ? `[${ip}]:${port}` : `${ip}:${port}`
}

function TupleDetail({
  title,
  tuple,
}: {
  title: string
  tuple: NormalizedFlowTuple
}) {
  return (
    <Paper variant="outlined" sx={{ p: 1.5, flex: 1, minWidth: 280 }}>
      <Stack spacing={1}>
        <Stack
          direction="row"
          spacing={1}
          sx={{ alignItems: 'center', justifyContent: 'space-between' }}
        >
          <Typography variant="subtitle2">{title}</Typography>
          <Chip
            size="small"
            color={tuple.complete ? 'success' : 'warning'}
            label={tuple.complete ? 'Complete' : 'Incomplete'}
          />
        </Stack>
        <Typography variant="body2" sx={{ fontFamily: 'monospace' }}>
          {endpoint(tuple.src_ip, tuple.src_port)}
        </Typography>
        <Typography variant="caption" color="text.secondary">
          to
        </Typography>
        <Typography variant="body2" sx={{ fontFamily: 'monospace' }}>
          {endpoint(tuple.dst_ip, tuple.dst_port)}
        </Typography>
        {tuple.dst_host && (
          <Typography variant="body2">Host: {tuple.dst_host}</Typography>
        )}
        <Divider />
        <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap' }}>
          <Chip
            size="small"
            variant="outlined"
            label={tuple.network.toUpperCase()}
          />
          <Chip size="small" variant="outlined" label={tuple.scope} />
          <Chip size="small" variant="outlined" label={tuple.source} />
          {tuple.shared && <Chip size="small" color="warning" label="Shared" />}
        </Stack>
      </Stack>
    </Paper>
  )
}

function isPacketCapture(artifact: SessionArtifact) {
  const value = `${artifact.name} ${artifact.kind} ${artifact.media_type}`
  const normalized = value.toLocaleLowerCase()
  return normalized.includes('pcap') || normalized.includes('packet')
}

export function TrafficTracerFlowDetail({
  flow,
  sessionArtifacts = emptyArtifacts,
  onClose,
}: TrafficTracerFlowDetailProps) {
  const packetCaptures = sessionArtifacts.filter(isPacketCapture)

  return (
    <Dialog open={flow !== null} onClose={onClose} fullWidth maxWidth="md">
      <DialogTitle>Normalized Flow details</DialogTitle>
      <DialogContent dividers>
        {flow && (
          <Stack spacing={2}>
            <Stack
              direction="row"
              spacing={1}
              sx={{ alignItems: 'center', flexWrap: 'wrap' }}
            >
              <Chip label={flow.protocol.toUpperCase()} size="small" />
              <Chip label={flow.match.status} size="small" />
              <Chip
                label={`${Math.round(flow.match.confidence * 100)}% confidence`}
                size="small"
              />
              {flow.shared && (
                <Chip color="warning" label="Shared Flow" size="small" />
              )}
            </Stack>

            {flow.shared && (
              <Alert severity="warning">
                This mapping shares a tuple or outer connection with another
                logical Flow. It is not an exclusive one-to-one association.
              </Alert>
            )}

            <Stack direction="row" spacing={1.5} sx={{ flexWrap: 'wrap' }}>
              <TupleDetail title="Pre-proxy tuple" tuple={flow.pre_flow} />
              {flow.post_flow ? (
                <TupleDetail title="Post-proxy tuple" tuple={flow.post_flow} />
              ) : (
                <Alert severity="warning" sx={{ flex: 1, minWidth: 280 }}>
                  No complete post-proxy tuple was recorded. The pre-proxy tuple
                  is not copied or inferred as a replacement.
                </Alert>
              )}
            </Stack>

            <Box>
              <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                Correlation
              </Typography>
              <Stack spacing={0.5}>
                <Typography variant="body2">
                  Reason: {flow.match.reason || '—'}
                </Typography>
                <Typography variant="body2">
                  Candidates: {flow.match.candidate_count}
                </Typography>
                <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>
                  Flow ID: {flow.flow_id}
                </Typography>
                <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>
                  Connection ID: {flow.conn_id || '—'}
                </Typography>
                <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>
                  Outer connection ID: {flow.outer_conn_id || '—'}
                </Typography>
              </Stack>
            </Box>

            {(flow.url || flow.resource_type || flow.relation) && (
              <Box>
                <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                  Request context
                </Typography>
                <Stack spacing={0.5}>
                  {flow.url && (
                    <Typography
                      variant="body2"
                      sx={{ overflowWrap: 'anywhere' }}
                    >
                      URL: {flow.url}
                    </Typography>
                  )}
                  {flow.resource_type && (
                    <Typography variant="body2">
                      Resource type: {flow.resource_type}
                    </Typography>
                  )}
                  {flow.relation && (
                    <Typography variant="body2">
                      Relation: {flow.relation}
                    </Typography>
                  )}
                </Stack>
              </Box>
            )}

            <Box>
              <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                Request IDs
              </Typography>
              {flow.request_ids.length > 0 ? (
                <Stack direction="row" spacing={0.75} sx={{ flexWrap: 'wrap' }}>
                  {flow.request_ids.map((requestId) => (
                    <Chip
                      key={requestId}
                      label={requestId}
                      size="small"
                      variant="outlined"
                    />
                  ))}
                </Stack>
              ) : (
                <Typography variant="body2" color="text.secondary">
                  No request IDs are associated with this Flow.
                </Typography>
              )}
            </Box>

            {packetCaptures.length > 0 && (
              <Box>
                <Typography variant="subtitle2">
                  Session packet captures
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  These artifacts belong to the Session; the schema does not
                  claim a per-Flow pcap association.
                </Typography>
                <TrafficTracerArtifactList
                  sessionId={flow.session_id}
                  artifacts={packetCaptures}
                />
              </Box>
            )}
          </Stack>
        )}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Close</Button>
      </DialogActions>
    </Dialog>
  )
}
