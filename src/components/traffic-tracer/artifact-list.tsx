import { OpenInNewRounded } from '@mui/icons-material'
import { Alert, Button, Chip, Divider, Stack, Typography } from '@mui/material'
import { useState } from 'react'

import { openTrafficTracerArtifact } from '@/services/cmds'
import type { SessionArtifact } from '@/types/traffic-tracer'

export interface TrafficTracerArtifactListProps {
  sessionId: string
  artifacts: SessionArtifact[]
}

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes < 0) return 'Unknown size'
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value >= 10 || unit === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`
}

export function TrafficTracerArtifactList({
  sessionId,
  artifacts,
}: TrafficTracerArtifactListProps) {
  const [openingName, setOpeningName] = useState<string | null>(null)
  const [openError, setOpenError] = useState<string | null>(null)

  const openArtifact = async (artifact: SessionArtifact) => {
    try {
      setOpenError(null)
      setOpeningName(artifact.name)
      await openTrafficTracerArtifact(sessionId, artifact.name)
    } catch (error) {
      setOpenError(String(error))
    } finally {
      setOpeningName(null)
    }
  }

  if (artifacts.length === 0) {
    return (
      <Typography color="text.secondary" sx={{ py: 2, textAlign: 'center' }}>
        This Session has no registered artifacts.
      </Typography>
    )
  }

  return (
    <Stack spacing={1}>
      {openError && <Alert severity="error">{openError}</Alert>}
      <Stack divider={<Divider flexItem />}>
        {artifacts.map((artifact) => (
          <Stack
            key={artifact.name}
            direction="row"
            spacing={1}
            sx={{
              alignItems: 'center',
              justifyContent: 'space-between',
              py: 1,
            }}
          >
            <Stack spacing={0.25} sx={{ minWidth: 0 }}>
              <Typography variant="body2" sx={{ fontWeight: 600 }}>
                {artifact.name}
              </Typography>
              <Stack direction="row" spacing={0.75} sx={{ flexWrap: 'wrap' }}>
                <Chip size="small" variant="outlined" label={artifact.kind} />
                <Chip
                  size="small"
                  variant="outlined"
                  label={artifact.media_type}
                />
                <Typography variant="caption" color="text.secondary">
                  {formatBytes(artifact.size_bytes)}
                </Typography>
              </Stack>
            </Stack>
            <Button
              size="small"
              startIcon={<OpenInNewRounded />}
              disabled={openingName !== null}
              onClick={() => void openArtifact(artifact)}
            >
              {openingName === artifact.name ? 'Opening…' : 'Open'}
            </Button>
          </Stack>
        ))}
      </Stack>
    </Stack>
  )
}
