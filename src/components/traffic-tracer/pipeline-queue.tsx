import { AddRounded, DeleteOutlineRounded } from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Checkbox,
  FormControlLabel,
  IconButton,
  Paper,
  Stack,
  TextField,
  Typography,
} from '@mui/material'
import { useState } from 'react'

import { useCurrentProxy } from '@/hooks/use-current-proxy'
import { useProfiles } from '@/hooks/use-profiles'
import { snapshotTrafficTracerPipelineCandidate } from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import type { PipelineCandidate } from '@/types/traffic-tracer'

import {
  PIPELINE_MAX_REPETITIONS,
  PIPELINE_MODE_STORAGE_KEY,
  PIPELINE_QUEUE_STORAGE_KEY,
  PIPELINE_REPETITIONS_STORAGE_KEY,
} from './pipeline-queue-storage'

interface Props {
  enabled: boolean
  candidates: PipelineCandidate[]
  repetitions: number
  targetCount: number
  disabled?: boolean
  onEnabledChange: (enabled: boolean) => void
  onRepetitionsChange: (repetitions: number) => void
  onChange: (candidates: PipelineCandidate[]) => void
}

export function TrafficTracerPipelineQueue({
  enabled,
  candidates,
  repetitions,
  targetCount,
  disabled = false,
  onEnabledChange,
  onRepetitionsChange,
  onChange,
}: Props) {
  const { current } = useProfiles()
  const { currentProxy, primaryGroupName } = useCurrentProxy()
  const [adding, setAdding] = useState(false)

  const update = (next: PipelineCandidate[]) => {
    localStorage.setItem(PIPELINE_QUEUE_STORAGE_KEY, JSON.stringify(next))
    onChange(next)
  }

  const addCurrent = async () => {
    if (!current?.uid || !primaryGroupName || !currentProxy?.name) return
    setAdding(true)
    try {
      const candidate = await snapshotTrafficTracerPipelineCandidate({
        profile_uid: current.uid,
        selection_group: primaryGroupName,
        requested_node: currentProxy.name,
      })
      const duplicate = candidates.some(
        (item) =>
          item.profile_uid === candidate.profile_uid &&
          item.selection_group === candidate.selection_group &&
          item.requested_node === candidate.requested_node,
      )
      if (duplicate) {
        showNotice.info('This Profile, selector and node is already queued.')
        return
      }
      update([...candidates, candidate])
    } catch (error) {
      showNotice.error(error)
    } finally {
      setAdding(false)
    }
  }

  return (
    <Paper variant="outlined" sx={{ p: 2, mb: 2 }}>
      <Stack spacing={1.25}>
        <FormControlLabel
          control={
            <Checkbox
              checked={enabled}
              disabled={disabled}
              onChange={(_, value) => {
                localStorage.setItem(
                  PIPELINE_MODE_STORAGE_KEY,
                  JSON.stringify(value),
                )
                onEnabledChange(value)
              }}
            />
          }
          label="Profile / node pipeline"
        />
        <Typography variant="body2" color="text.secondary">
          Queue the effective Profile, selector and concrete node currently
          active in Mihomo. Each entry runs the selected sites serially; entries
          run in the order shown.
        </Typography>
        {enabled && (
          <>
            <Stack direction="row" spacing={1.5} sx={{ alignItems: 'center' }}>
              <TextField
                size="small"
                type="number"
                label="Repetitions per node"
                value={repetitions}
                disabled={disabled}
                slotProps={{
                  htmlInput: { min: 1, max: PIPELINE_MAX_REPETITIONS },
                }}
                onChange={(event) => {
                  const value = Number(event.target.value)
                  if (
                    !Number.isInteger(value) ||
                    value < 1 ||
                    value > PIPELINE_MAX_REPETITIONS
                  )
                    return
                  localStorage.setItem(
                    PIPELINE_REPETITIONS_STORAGE_KEY,
                    String(value),
                  )
                  onRepetitionsChange(value)
                }}
                sx={{ width: 190 }}
              />
              <Typography variant="body2" color="text.secondary">
                {candidates.length * repetitions} batches ·{' '}
                {candidates.length * repetitions * targetCount} baseline
                Sessions · strict serial order
              </Typography>
            </Stack>
            <Alert severity="info">
              Repetitions are independent full sites.yaml samples. A URL-level
              application retry remains a separate recovery attempt inside one
              batch.
            </Alert>
            <Stack
              direction="row"
              spacing={1}
              sx={{ alignItems: 'center', justifyContent: 'space-between' }}
            >
              <Typography variant="subtitle2">
                Pipeline nodes ({candidates.length})
              </Typography>
              <Button
                size="small"
                startIcon={<AddRounded />}
                disabled={
                  disabled ||
                  adding ||
                  !current?.uid ||
                  !primaryGroupName ||
                  !currentProxy?.name
                }
                onClick={() => void addCurrent()}
              >
                Add current pair
              </Button>
            </Stack>
            {candidates.length === 0 ? (
              <Alert severity="info">
                Activate a Profile and concrete selector node, then add it to
                the queue. Repeat for every sample node.
              </Alert>
            ) : (
              <Box
                sx={{
                  maxHeight: 220,
                  overflowY: 'auto',
                  border: 1,
                  borderColor: 'divider',
                  borderRadius: 1,
                }}
              >
                {candidates.map((candidate, index) => (
                  <Stack
                    key={`${candidate.profile_uid}\u0000${candidate.selection_group}\u0000${candidate.requested_node}`}
                    direction="row"
                    spacing={1}
                    sx={{
                      alignItems: 'center',
                      px: 1.25,
                      minHeight: 46,
                      borderBottom: index + 1 < candidates.length ? 1 : 0,
                      borderColor: 'divider',
                    }}
                  >
                    <Typography variant="body2" sx={{ width: 28 }}>
                      {index + 1}.
                    </Typography>
                    <Box sx={{ minWidth: 0, flex: 1 }}>
                      <Typography variant="body2" noWrap>
                        {candidate.profile_uid} · {candidate.selection_group} ·{' '}
                        {candidate.requested_node}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        effective config{' '}
                        {candidate.profile_fingerprint.slice(0, 12)}
                      </Typography>
                    </Box>
                    <IconButton
                      size="small"
                      disabled={disabled}
                      aria-label={`Remove pipeline node ${index + 1}`}
                      onClick={() =>
                        update(
                          candidates.filter(
                            (_, position) => position !== index,
                          ),
                        )
                      }
                    >
                      <DeleteOutlineRounded fontSize="small" />
                    </IconButton>
                  </Stack>
                ))}
              </Box>
            )}
          </>
        )}
      </Stack>
    </Paper>
  )
}
