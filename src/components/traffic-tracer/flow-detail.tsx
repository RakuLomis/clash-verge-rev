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
import { useTranslation } from 'react-i18next'

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
  const { t } = useTranslation()
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
            label={
              tuple.complete
                ? t('settings.trafficTracer.common.states.complete')
                : t('settings.trafficTracer.common.states.incomplete')
            }
          />
        </Stack>
        <Typography variant="body2" sx={{ fontFamily: 'monospace' }}>
          {endpoint(tuple.src_ip, tuple.src_port)}
        </Typography>
        <Typography variant="caption" color="text.secondary">
          {t('settings.trafficTracer.flows.detail.to')}
        </Typography>
        <Typography variant="body2" sx={{ fontFamily: 'monospace' }}>
          {endpoint(tuple.dst_ip, tuple.dst_port)}
        </Typography>
        {tuple.dst_host && (
          <Typography variant="body2">
            {t('settings.trafficTracer.flows.detail.host')}: {tuple.dst_host}
          </Typography>
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
          {tuple.shared && (
            <Chip
              size="small"
              color="warning"
              label={t('settings.trafficTracer.flows.shared')}
            />
          )}
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
  const { t } = useTranslation()
  const packetCaptures = sessionArtifacts.filter(isPacketCapture)

  return (
    <Dialog open={flow !== null} onClose={onClose} fullWidth maxWidth="md">
      <DialogTitle>
        {t('settings.trafficTracer.flows.detail.title')}
      </DialogTitle>
      <DialogContent dividers>
        {flow && (
          <Stack spacing={2}>
            <Stack
              direction="row"
              spacing={1}
              sx={{ alignItems: 'center', flexWrap: 'wrap' }}
            >
              <Chip label={flow.protocol.toUpperCase()} size="small" />
              <Chip
                label={t(
                  `settings.trafficTracer.flows.match.${flow.match.status}`,
                )}
                size="small"
              />
              <Chip
                label={t('settings.trafficTracer.flows.detail.confidence', {
                  value: Math.round(flow.match.confidence * 100),
                })}
                size="small"
              />
              {flow.shared && (
                <Chip
                  color="warning"
                  label={t('settings.trafficTracer.flows.detail.sharedFlow')}
                  size="small"
                />
              )}
            </Stack>

            {flow.shared && (
              <Alert severity="warning">
                {t('settings.trafficTracer.flows.detail.sharedWarning')}
              </Alert>
            )}

            <Stack direction="row" spacing={1.5} sx={{ flexWrap: 'wrap' }}>
              <TupleDetail
                title={t('settings.trafficTracer.flows.detail.preTuple')}
                tuple={flow.pre_flow}
              />
              {flow.post_flow ? (
                <TupleDetail
                  title={t('settings.trafficTracer.flows.detail.postTuple')}
                  tuple={flow.post_flow}
                />
              ) : (
                <Alert severity="warning" sx={{ flex: 1, minWidth: 280 }}>
                  {t('settings.trafficTracer.flows.detail.missingPost')}
                </Alert>
              )}
            </Stack>

            <Box>
              <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                {t('settings.trafficTracer.flows.detail.correlation')}
              </Typography>
              <Stack spacing={0.5}>
                <Typography variant="body2">
                  {t('settings.trafficTracer.flows.detail.reason')}:{' '}
                  {flow.match.reason || '—'}
                </Typography>
                <Typography variant="body2">
                  {t('settings.trafficTracer.flows.detail.candidates')}:{' '}
                  {flow.match.candidate_count}
                </Typography>
                <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>
                  {t('settings.trafficTracer.flows.detail.flowId')}:{' '}
                  {flow.flow_id}
                </Typography>
                <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>
                  {t('settings.trafficTracer.flows.detail.connectionId')}:{' '}
                  {flow.conn_id || '—'}
                </Typography>
                <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>
                  {t('settings.trafficTracer.flows.detail.outerConnectionId')}:{' '}
                  {flow.outer_conn_id || '—'}
                </Typography>
              </Stack>
            </Box>

            {(flow.url || flow.resource_type || flow.relation) && (
              <Box>
                <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                  {t('settings.trafficTracer.flows.detail.requestContext')}
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
                      {t('settings.trafficTracer.flows.detail.resourceType')}:{' '}
                      {flow.resource_type}
                    </Typography>
                  )}
                  {flow.relation && (
                    <Typography variant="body2">
                      {t('settings.trafficTracer.flows.detail.relation')}:{' '}
                      {flow.relation}
                    </Typography>
                  )}
                </Stack>
              </Box>
            )}

            <Box>
              <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
                {t('settings.trafficTracer.flows.detail.requestIds')}
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
                  {t('settings.trafficTracer.flows.detail.noRequestIds')}
                </Typography>
              )}
            </Box>

            {packetCaptures.length > 0 && (
              <Box>
                <Typography variant="subtitle2">
                  {t('settings.trafficTracer.flows.detail.packetCaptures')}
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  {t('settings.trafficTracer.flows.detail.packetCaptureScope')}
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
        <Button onClick={onClose}>
          {t('settings.trafficTracer.common.actions.close')}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
