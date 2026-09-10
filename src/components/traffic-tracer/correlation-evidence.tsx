import { Alert, Box, Stack, Typography } from '@mui/material'

import type {
  LayeredCoverage,
  RequestIndexRecord,
} from '@/types/traffic-tracer'

export function CorrelationEvidence({
  coverage,
  requests,
}: {
  coverage?: LayeredCoverage
  requests: RequestIndexRecord[]
}) {
  if (!coverage) return null
  const page = coverage.page_attributed
  const ambiguous = (page?.browser_requests ?? coverage.browser_requests)
    .ambiguous
  const tail =
    coverage.capture_global?.core_logical_flows.capture_tail_unattributed
  const missing = page?.logical_flows.unexpected_missing
  const examples = requests
    .filter((request) => request.attribution.status === 'ambiguous')
    .slice(0, 5)
  if (!ambiguous && !tail && !missing) return null
  return (
    <Box
      component="details"
      sx={{ border: 1, borderColor: 'divider', p: 1.5, borderRadius: 1 }}
    >
      <Typography component="summary" sx={{ cursor: 'pointer' }}>
        Correlation evidence · {ambiguous} ambiguous requests ·{' '}
        {tail ?? 'unknown'} background tail flows
      </Typography>
      <Stack spacing={1} sx={{ mt: 1, maxHeight: 240, overflowY: 'auto' }}>
        {ambiguous > 0 && (
          <Alert severity="warning">
            A request with multiple candidate connections is not a confirmed
            URL-to-flow match. Candidates are retained; none is selected by
            guesswork.
          </Alert>
        )}
        {(tail ?? 0) > 0 && (
          <Alert severity="info">
            Capture-global tail evidence: {tail}. These flows are not attributed
            to this page. This count alone does not establish missing page
            traffic or a proxy failure.
          </Alert>
        )}
        <Typography variant="body2">
          Page-attributed unexpected missing endpoints:{' '}
          {missing ?? 'not reported'}. Background counts are not added to this
          page count.
        </Typography>
        {ambiguous > 0 && (
          <Typography variant="caption">
            Showing {examples.length} examples from loaded request records (up
            to 5), not the full dataset.
          </Typography>
        )}
        {examples.map((request) => (
          <Box key={request.request_id} sx={{ overflowWrap: 'anywhere' }}>
            <Typography variant="body2">{request.url}</Typography>
            <Typography variant="caption" component="div">
              Reason:{' '}
              {request.attribution.unmatched_reason ??
                request.attribution.method}
              {' · '}Candidates:{' '}
              {request.candidate_connection_ids.join(', ') || 'not reported'}
            </Typography>
            <Typography variant="caption" component="div">
              Evidence:{' '}
              {request.attribution.evidence.join(', ') || 'not reported'}
            </Typography>
          </Box>
        ))}
      </Stack>
    </Box>
  )
}
