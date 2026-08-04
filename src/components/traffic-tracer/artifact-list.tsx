import { OpenInNewRounded } from '@mui/icons-material'
import { Alert, Button, Chip, Divider, Stack, Typography } from '@mui/material'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { openTrafficTracerArtifact } from '@/services/cmds'
import type { SessionArtifact } from '@/types/traffic-tracer'

export interface TrafficTracerArtifactListProps {
  sessionId: string
  artifacts: SessionArtifact[]
}

function formatBytes(bytes: number, unknownSize: string) {
  if (!Number.isFinite(bytes) || bytes < 0) return unknownSize
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
  const { t } = useTranslation()
  const [openingName, setOpeningName] = useState<string | null>(null)
  const [openError, setOpenError] = useState<string | null>(null)

  const openArtifact = async (artifact: SessionArtifact) => {
    const artifactId = artifact.artifact_id ?? artifact.name
    try {
      setOpenError(null)
      setOpeningName(artifactId)
      await openTrafficTracerArtifact(sessionId, artifactId)
    } catch (error) {
      setOpenError(String(error))
    } finally {
      setOpeningName(null)
    }
  }

  if (artifacts.length === 0) {
    return (
      <Typography color="text.secondary" sx={{ py: 2, textAlign: 'center' }}>
        {t('settings.trafficTracer.sessions.artifactsEmpty')}
      </Typography>
    )
  }

  return (
    <Stack spacing={1}>
      {openError && <Alert severity="error">{openError}</Alert>}
      <Stack divider={<Divider flexItem />}>
        {artifacts.map((artifact) => (
          <Stack
            key={artifact.artifact_id ?? artifact.path}
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
                <Chip
                  size="small"
                  variant="outlined"
                  label={artifact.role ?? artifact.kind ?? 'other'}
                />
                <Chip
                  size="small"
                  variant="outlined"
                  label={artifact.media_type}
                />
                <Typography variant="caption" color="text.secondary">
                  {formatBytes(
                    artifact.size_bytes,
                    t('settings.trafficTracer.sessions.unknownSize'),
                  )}
                </Typography>
              </Stack>
            </Stack>
            <Button
              size="small"
              startIcon={<OpenInNewRounded />}
              disabled={openingName !== null}
              onClick={() => void openArtifact(artifact)}
            >
              {openingName === (artifact.artifact_id ?? artifact.name)
                ? t('settings.trafficTracer.common.progress.opening')
                : t('settings.trafficTracer.common.actions.open')}
            </Button>
          </Stack>
        ))}
      </Stack>
    </Stack>
  )
}
