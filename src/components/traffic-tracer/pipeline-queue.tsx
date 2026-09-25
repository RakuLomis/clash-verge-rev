import { AddRounded, DeleteOutlineRounded } from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Checkbox,
  FormControlLabel,
  IconButton,
  MenuItem,
  Paper,
  Stack,
  TextField,
  Typography,
} from '@mui/material'
import { useMemo, useState } from 'react'

import { useProfiles } from '@/hooks/use-profiles'
import { useProxiesData } from '@/providers/app-data-context'
import { snapshotTrafficTracerPipelineCandidate } from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import type { PipelineCandidate } from '@/types/traffic-tracer'

import {
  PIPELINE_MAX_REPETITIONS,
  PIPELINE_MODE_STORAGE_KEY,
  PIPELINE_QUEUE_STORAGE_KEY,
  PIPELINE_REPETITIONS_STORAGE_KEY,
} from './pipeline-queue-storage'
import {
  pipelineCandidateIdentity,
  pipelineCandidateRuntimeIssue,
  pipelineSelectorGroups,
} from './pipeline-selector'

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
  const { proxies } = useProxiesData()
  const [adding, setAdding] = useState(false)
  const [selectedSelectors, setSelectedSelectors] = useState<
    Record<string, string>
  >({})
  const selectors = useMemo(() => pipelineSelectorGroups(proxies), [proxies])
  const rememberedSelector = current?.uid
    ? selectedSelectors[current.uid]
    : undefined
  const selectedSelector =
    selectors.find((group) => group.name === rememberedSelector) ??
    (selectors.length === 1 ? selectors[0] : null)
  const selectionGroup = selectedSelector?.name ?? ''
  const requestedNode = selectedSelector?.now ?? ''
  const currentProxy = requestedNode
    ? proxies?.records?.[requestedNode]
    : undefined
  const currentIssue =
    selectionGroup && requestedNode
      ? pipelineCandidateRuntimeIssue(proxies, selectionGroup, requestedNode)
      : null
  const invalidCurrentCandidates = proxies
    ? candidates.filter(
        (candidate) =>
          candidate.profile_uid === current?.uid &&
          pipelineCandidateRuntimeIssue(
            proxies,
            candidate.selection_group,
            candidate.requested_node,
          ) !== null,
      )
    : []

  const update = (next: PipelineCandidate[]) => {
    localStorage.setItem(PIPELINE_QUEUE_STORAGE_KEY, JSON.stringify(next))
    onChange(next)
  }

  const addCurrent = async () => {
    if (!current?.uid || !selectionGroup || !requestedNode || currentIssue)
      return
    setAdding(true)
    try {
      const candidate = await snapshotTrafficTracerPipelineCandidate({
        profile_uid: current.uid,
        selection_group: selectionGroup,
        requested_node: requestedNode,
      })
      const duplicate = candidates.some(
        (item) =>
          pipelineCandidateIdentity(item) ===
          pipelineCandidateIdentity(candidate),
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
          label="Profile / concrete node pipeline"
        />
        <Typography variant="body2" color="text.secondary">
          Queue effective Profile and node pairs. Each repetition captures in
          target order; every target uses one frozen candidate order. Analysis
          starts only after that repetition's entire capture wave finishes.
        </Typography>
        {enabled && (
          <>
            <Stack direction="row" spacing={1.5} sx={{ alignItems: 'center' }}>
              <TextField
                select
                size="small"
                label="Pipeline selector"
                value={selectedSelector?.name ?? ''}
                disabled={disabled || selectors.length === 0}
                onChange={(event) => {
                  if (!current?.uid) return
                  setSelectedSelectors((existing) => ({
                    ...existing,
                    [current.uid]: event.target.value,
                  }))
                }}
                sx={{ minWidth: 230 }}
              >
                {selectors.length > 1 && (
                  <MenuItem value="" disabled>
                    Select a manual selector
                  </MenuItem>
                )}
                {selectors.map((selector) => (
                  <MenuItem key={selector.name} value={selector.name}>
                    {selector.name}
                  </MenuItem>
                ))}
              </TextField>
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="body2" noWrap>
                  Current node: {requestedNode || 'not selected'}
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  {currentProxy?.type
                    ? `Runtime type: ${currentProxy.type}`
                    : 'Choose a concrete node in the selected group.'}
                </Typography>
              </Box>
            </Stack>
            {selectors.length === 0 && (
              <Alert severity="error">
                No manual Selector is available in the active Profile. URLTest,
                Fallback and LoadBalance groups cannot define a reproducible
                pipeline candidate.
              </Alert>
            )}
            {selectors.length > 1 && !selectedSelector && (
              <Alert severity="warning">
                This Profile has multiple manual selectors. Choose the one
                TrafficTracer should freeze before adding a node.
              </Alert>
            )}
            {currentIssue && <Alert severity="warning">{currentIssue}</Alert>}
            {invalidCurrentCandidates.length > 0 && (
              <Alert
                severity="error"
                action={
                  <Button
                    size="small"
                    color="inherit"
                    disabled={disabled}
                    onClick={() => {
                      const invalid = new Set(
                        invalidCurrentCandidates.map(pipelineCandidateIdentity),
                      )
                      update(
                        candidates.filter(
                          (candidate) =>
                            !invalid.has(pipelineCandidateIdentity(candidate)),
                        ),
                      )
                    }}
                  >
                    Remove invalid
                  </Button>
                }
              >
                {invalidCurrentCandidates.length} stored candidate(s) use an
                automatic or unavailable group and cannot be started safely.
              </Alert>
            )}
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
                {candidates.length * repetitions * targetCount} capture cells ·
                repetition → target → candidate · capture wave, then analysis
              </Typography>
            </Stack>
            <Alert severity="info">
              The candidate order is balanced and reproducibly frozen in the
              pipeline manifest. It stays identical for all targets in one
              repetition and rotates between repetitions. Every candidate is
              rebound to its semantic runtime configuration before capture.
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
                  !selectionGroup ||
                  !requestedNode ||
                  Boolean(currentIssue) ||
                  invalidCurrentCandidates.length > 0
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
                    key={pipelineCandidateIdentity(candidate)}
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
                        {candidate.profile_fingerprint_kind ===
                        'runtime_semantic_v2'
                          ? 'semantic snapshot'
                          : 'legacy snapshot · will rebind'}{' '}
                        {candidate.profile_fingerprint.slice(0, 12)}
                        {candidate.recorded_at &&
                          ` · ${new Date(candidate.recorded_at).toLocaleString()}`}
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
