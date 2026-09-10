import { Box, Stack, Typography } from '@mui/material'

import type { PipelineManifest } from '@/types/traffic-tracer'

/** Use the backend's same-snapshot aggregate, never rescan Session directories. */
export default function PipelineQualityOverview({
  pipeline,
}: {
  pipeline: PipelineManifest
}) {
  const aggregate = pipeline.aggregate
  if (
    !aggregate ||
    aggregate.pipeline_id !== pipeline.pipeline_id ||
    aggregate.updated_at !== pipeline.updated_at
  ) {
    return <Box>Whole pipeline quality: awaiting current summary</Box>
  }
  const candidates = aggregate.candidates
  const total = (
    key: 'completed' | 'degraded' | 'failed' | 'interrupted' | 'cancelled',
  ) => candidates.reduce((sum, candidate) => sum + candidate[key], 0)
  const terminalNodes = candidates.filter(
    (c) => c.cells_planned > 0 && c.cells_terminal === c.cells_planned,
  ).length
  return (
    <Stack spacing={0.5} sx={{ mt: 1 }}>
      <Typography variant="subtitle2">Whole pipeline summary</Typography>
      <Box>
        Tasks finished {aggregate.terminal_cells}/{aggregate.planned_cells} ·
        Nodes finished {terminalNodes}/{candidates.length}
      </Box>
      <Box>
        {total('completed')} fully passed · {total('degraded')} degraded ·{' '}
        {total('failed')} failed/skipped · {total('interrupted')} interrupted ·{' '}
        {total('cancelled')} cancelled
      </Box>
      {(['capture_integrity', 'correlation', 'application'] as const).map(
        (plane) => (
          <Box key={plane}>
            {plane.replaceAll('_', ' ')}:{' '}
            {(
              [
                'passed',
                'degraded',
                'failed',
                'indeterminate',
                'not_applicable',
              ] as const
            )
              .map(
                (state) =>
                  `${candidates.reduce((sum, c) => sum + c[plane][state], 0)} ${state.replaceAll('_', ' ')}`,
              )
              .join(' · ')}
          </Box>
        ),
      )}
      <Box sx={{ opacity: 0.7 }}>
        Quality counts cover final Sessions only; earlier retry attempts are
        excluded. Finished does not mean passed.
      </Box>
    </Stack>
  )
}
