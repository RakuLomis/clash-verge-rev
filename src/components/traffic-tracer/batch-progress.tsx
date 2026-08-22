import {
  Alert,
  Box,
  Button,
  Chip,
  Collapse,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  LinearProgress,
  Paper,
  Stack,
  Typography,
} from '@mui/material'
import { useEffect, useRef, useState } from 'react'

import type { BatchStatusResult } from '@/types/traffic-tracer'

import { TrafficTracerSessionDetail } from './session-detail'

export function TrafficTracerBatchProgress({
  status,
  workspaceRoot,
  interrupting = false,
  canInterrupt = false,
  viewingHistory = false,
  resuming = false,
  onInterrupt,
  onResume,
}: {
  status: BatchStatusResult
  workspaceRoot: string
  interrupting?: boolean
  canInterrupt?: boolean
  viewingHistory?: boolean
  resuming?: boolean
  onInterrupt: () => void
  onResume: () => void
}) {
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [confirmOpen, setConfirmOpen] = useState(false)
  const { batch } = status
  const terminal = ['completed', 'failed', 'cancelled', 'interrupted'].includes(
    batch.state,
  )
  const detailsKey = `${batch.batch_id}:${terminal}`
  const [detailsOverride, setDetailsOverride] = useState<{
    key: string
    open: boolean
  } | null>(null)
  const detailsOpen =
    detailsOverride?.key === detailsKey ? detailsOverride.open : !terminal
  const currentRowRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    if (!detailsOpen || terminal) return
    const row = currentRowRef.current
    if (typeof row?.scrollIntoView === 'function') {
      row.scrollIntoView({ block: 'nearest' })
    }
  }, [batch.current_index, detailsOpen, terminal])
  const position = batch.current_index ?? batch.resume.next_index
  const completed = batch.children.filter(
    (child) => child.state === 'completed',
  ).length
  const progress = batch.targets.length
    ? (completed / batch.targets.length) * 100
    : 0
  const canResume = batch.state === 'failed' || batch.state === 'interrupted'
  return (
    <Paper
      variant="outlined"
      sx={{ p: 2 }}
      data-testid="traffic-tracer-batch-progress"
    >
      <Stack spacing={1.5}>
        <Stack
          direction="row"
          sx={{ justifyContent: 'space-between', alignItems: 'center' }}
        >
          <Box>
            <Typography variant="h6" sx={{ fontSize: 17, fontWeight: 600 }}>
              Capture group
            </Typography>
            <Typography variant="body2" color="text.secondary">
              Target {Math.min(position + 1, batch.targets.length)}/
              {batch.targets.length} · {batch.stage}
            </Typography>
          </Box>
          <Chip
            label={batch.state}
            color={
              batch.state === 'failed'
                ? 'error'
                : batch.state === 'completed'
                  ? 'success'
                  : 'default'
            }
          />
        </Stack>
        <LinearProgress variant="determinate" value={progress} />
        {(batch.cancel_requested || status.job?.interrupt_requested) && (
          <Alert severity="info">
            {status.job?.interrupt_requested
              ? 'Interruption requested. Safely closing the current Chrome and capture processes before the group stops.'
              : 'Cancellation requested. The current Chrome cleanup must finish before the capture group stops.'}
          </Alert>
        )}
        <Stack
          direction="row"
          sx={{ alignItems: 'center', justifyContent: 'space-between' }}
        >
          <Typography variant="body2" color="text.secondary">
            {completed} completed · {batch.targets.length - completed} remaining
          </Typography>
          <Button
            size="small"
            onClick={() =>
              setDetailsOverride({ key: detailsKey, open: !detailsOpen })
            }
          >
            {detailsOpen ? 'Hide targets' : 'Show targets'}
          </Button>
        </Stack>
        <Collapse in={detailsOpen}>
          <Box
            sx={{
              maxHeight: 240,
              overflowY: 'auto',
              border: 1,
              borderColor: 'divider',
              borderRadius: 1,
              p: 0.75,
            }}
          >
            <Stack spacing={0.5}>
              {batch.children.map((child, index) => {
                const target = batch.targets[index]
                return (
                  <Stack
                    key={target.index}
                    ref={index === batch.current_index ? currentRowRef : null}
                    direction="row"
                    spacing={1}
                    sx={{
                      alignItems: 'center',
                      minHeight: 32,
                      px: 0.5,
                      borderRadius: 0.5,
                      bgcolor:
                        index === batch.current_index
                          ? 'action.selected'
                          : 'transparent',
                    }}
                  >
                    <Chip
                      size="small"
                      label={child.state}
                      sx={{ minWidth: 82 }}
                    />
                    <Typography
                      variant="body2"
                      title={`${target.domain} — ${target.url}`}
                      sx={{
                        flex: 1,
                        minWidth: 0,
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {index + 1}. {target.domain} — {target.url}
                    </Typography>
                    {child.error && (
                      <Typography
                        variant="caption"
                        color="error"
                        title={`${child.error.code}: ${child.error.message}`}
                        sx={{
                          maxWidth: 220,
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {child.error.code}
                      </Typography>
                    )}
                    {child.session_id && (
                      <Button
                        size="small"
                        onClick={() => setSessionId(child.session_id)}
                      >
                        Analysis
                      </Button>
                    )}
                  </Stack>
                )
              })}
            </Stack>
          </Box>
        </Collapse>
        <Stack direction="row" spacing={1} sx={{ justifyContent: 'flex-end' }}>
          {batch.state === 'running' && canInterrupt && !viewingHistory && (
            <Button
              color="warning"
              disabled={
                interrupting ||
                batch.cancel_requested ||
                status.job?.interrupt_requested
              }
              onClick={() => setConfirmOpen(true)}
            >
              {interrupting || status.job?.interrupt_requested
                ? 'Interrupting…'
                : 'Interrupt current capture'}
            </Button>
          )}
          {canResume && (
            <Button variant="contained" disabled={resuming} onClick={onResume}>
              {resuming
                ? 'Resuming…'
                : batch.state === 'interrupted'
                  ? 'Resume from interrupted target'
                  : 'Resume from failed target'}
            </Button>
          )}
        </Stack>
      </Stack>
      <TrafficTracerSessionDetail
        sessionId={sessionId}
        workspaceRoot={workspaceRoot}
        onClose={() => setSessionId(null)}
      />
      <Dialog open={confirmOpen} onClose={() => setConfirmOpen(false)}>
        <DialogTitle>Interrupt current capture?</DialogTitle>
        <DialogContent>
          <DialogContentText>
            Completed targets will be preserved. The current target will stop
            after Chrome and capture cleanup, later targets will not start, and
            Resume will retry the interrupted target in this capture group.
          </DialogContentText>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setConfirmOpen(false)}>Keep running</Button>
          <Button
            color="warning"
            variant="contained"
            onClick={() => {
              setConfirmOpen(false)
              onInterrupt()
            }}
          >
            Interrupt capture
          </Button>
        </DialogActions>
      </Dialog>
    </Paper>
  )
}
