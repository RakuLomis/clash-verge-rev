import {
  Alert,
  Box,
  Button,
  Chip,
  LinearProgress,
  Paper,
  Stack,
  Typography,
} from '@mui/material'
import { useState } from 'react'

import type { BatchStatusResult } from '@/types/traffic-tracer'

import { TrafficTracerSessionDetail } from './session-detail'

export function TrafficTracerBatchProgress({
  status,
  workspaceRoot,
  cancelling = false,
  resuming = false,
  onCancel,
  onResume,
}: {
  status: BatchStatusResult
  workspaceRoot: string
  cancelling?: boolean
  resuming?: boolean
  onCancel: () => void
  onResume: () => void
}) {
  const [sessionId, setSessionId] = useState<string | null>(null)
  const { batch } = status
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
        {batch.cancel_requested && (
          <Alert severity="info">
            Cancellation requested. The current Chrome cleanup must finish
            before the capture group stops.
          </Alert>
        )}
        <Stack spacing={0.75}>
          {batch.children.map((child, index) => {
            const target = batch.targets[index]
            return (
              <Stack
                key={target.index}
                direction="row"
                spacing={1}
                sx={{ alignItems: 'center' }}
              >
                <Chip size="small" label={child.state} />
                <Typography
                  variant="body2"
                  sx={{ flex: 1, overflowWrap: 'anywhere' }}
                >
                  {index + 1}. {target.domain} — {target.url}
                  {child.error
                    ? ` · ${child.error.code}: ${child.error.message}`
                    : ''}
                </Typography>
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
        <Stack direction="row" spacing={1} sx={{ justifyContent: 'flex-end' }}>
          {batch.state === 'running' && (
            <Button
              color="warning"
              disabled={cancelling || batch.cancel_requested}
              onClick={onCancel}
            >
              {cancelling ? 'Cancelling…' : 'Cancel capture group'}
            </Button>
          )}
          {canResume && (
            <Button variant="contained" disabled={resuming} onClick={onResume}>
              {resuming ? 'Resuming…' : 'Resume from failed target'}
            </Button>
          )}
        </Stack>
      </Stack>
      <TrafficTracerSessionDetail
        sessionId={sessionId}
        workspaceRoot={workspaceRoot}
        onClose={() => setSessionId(null)}
      />
    </Paper>
  )
}
