import { PlayArrowRounded, SearchRounded } from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Checkbox,
  FormControlLabel,
  InputAdornment,
  MenuItem,
  Paper,
  Stack,
  TextField,
  Typography,
} from '@mui/material'
import { appDataDir, join } from '@tauri-apps/api/path'
import { open } from '@tauri-apps/plugin-dialog'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router'

import { getNetworkInterfaces } from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import type {
  CaptureStartRequest,
  CompleteEnvironmentReport,
  DiagnosticCheck,
  EnvironmentRequest,
} from '@/types/traffic-tracer'

import {
  captureRequestFromDraft,
  defaultCaptureFormDraft,
  deriveDomain,
  environmentRequestFromDraft,
  environmentRequestsEqual,
  isAbsolutePlatformPath,
  suggestCaptureInterfaces,
  validateCaptureForm,
  type CaptureFormDraft,
} from './capture-form-model'
import { TrafficTracerEnvironmentCard } from './environment-card'
import type { EnvironmentRemediationTarget } from './environment-model'

const STORAGE_KEY = 'traffictracer.captureForm.v1'

export interface TrafficTracerCaptureFormProps {
  environment?: CompleteEnvironmentReport
  diagnosticRequest?: EnvironmentRequest | null
  diagnosing?: boolean
  diagnosticError?: unknown
  captureLocked?: boolean
  submitting?: boolean
  onDiagnose: (request: EnvironmentRequest) => void
  onRetryDiagnostics?: () => void
  onSubmit: (request: CaptureStartRequest) => Promise<unknown> | void
}

function restoredDraft(): CaptureFormDraft {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (!stored) return defaultCaptureFormDraft
    const parsed = JSON.parse(stored) as Partial<CaptureFormDraft>
    return {
      ...defaultCaptureFormDraft,
      ...parsed,
      options: { ...defaultCaptureFormDraft.options, ...parsed.options },
    }
  } catch {
    return defaultCaptureFormDraft
  }
}

export function TrafficTracerCaptureForm({
  environment,
  diagnosticRequest,
  diagnosing = false,
  diagnosticError,
  captureLocked = false,
  submitting = false,
  onDiagnose,
  onRetryDiagnostics,
  onSubmit,
}: TrafficTracerCaptureFormProps) {
  const navigate = useNavigate()
  const [draft, setDraft] = useState(restoredDraft)
  const [interfaces, setInterfaces] = useState<string[]>([])
  const [submitted, setSubmitted] = useState(false)
  const tunRef = useRef<HTMLInputElement>(null)
  const physicalRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    let disposed = false
    Promise.all([getNetworkInterfaces(), appDataDir()])
      .then(async ([available, dataDir]) => {
        if (disposed) return
        setInterfaces(available)
        const suggested = suggestCaptureInterfaces(available)
        const outputRoot = await join(dataDir, 'traffictracer-sessions')
        if (disposed) return
        setDraft((current) => ({
          ...current,
          tun_interface:
            current.tun_interface ||
            suggested.tun ||
            defaultCaptureFormDraft.tun_interface,
          physical_interface: current.physical_interface || suggested.physical,
          output_root: current.output_root || outputRoot,
        }))
      })
      .catch(console.error)
    return () => {
      disposed = true
    }
  }, [])

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(draft))
  }, [draft])

  useEffect(() => {
    const chromeCheck = environment?.checks.find(
      (check) => check.code === 'CHROME_READY',
    )
    if (
      typeof chromeCheck?.details !== 'object' ||
      chromeCheck.details === null
    ) {
      return
    }
    const path = (chromeCheck.details as Record<string, unknown>).path
    if (typeof path === 'string' && isAbsolutePlatformPath(path)) {
      // The diagnostic resolves a PATH command to the executable required by capture.
      // eslint-disable-next-line @eslint-react/set-state-in-effect
      setDraft((current) =>
        current.chrome_binary === path
          ? current
          : { ...current, chrome_binary: path },
      )
    }
  }, [environment])

  const errors = useMemo(() => validateCaptureForm(draft), [draft])
  const environmentRequest = useMemo(
    () => environmentRequestFromDraft(draft),
    [draft],
  )
  const diagnosticsCurrent = environmentRequestsEqual(
    diagnosticRequest,
    environmentRequest,
  )
  const blocking = !environment || !environment.ok || !diagnosticsCurrent
  const disabled =
    submitting ||
    captureLocked ||
    diagnosing ||
    blocking ||
    Object.keys(errors).length > 0

  const update = <Key extends keyof CaptureFormDraft>(
    key: Key,
    value: CaptureFormDraft[Key],
  ) => setDraft((current) => ({ ...current, [key]: value }))

  const updateOption = (
    key: keyof CaptureFormDraft['options'],
    value: boolean,
  ) =>
    setDraft((current) => ({
      ...current,
      options: { ...current.options, [key]: value },
    }))

  const pickChrome = async () => {
    const selected = await open({
      directory: false,
      multiple: false,
      title: 'Select Chrome or Chromium',
    })
    if (selected) update('chrome_binary', String(selected))
  }

  const pickOutput = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: 'Select TrafficTracer Session directory',
    })
    if (selected) update('output_root', String(selected))
  }

  const handleRemediation = (
    target: EnvironmentRemediationTarget,
    check: DiagnosticCheck,
  ) => {
    if (target === 'core' || target === 'tun' || target === 'tun-service') {
      navigate('/settings')
    } else if (target === 'interfaces') {
      const ref = check.code.startsWith('TUN_INTERFACE_') ? tunRef : physicalRef
      ref.current?.focus()
    } else if (target === 'chrome') {
      void pickChrome()
    } else if (target === 'output') {
      void pickOutput()
    } else if (target === 'diagnostics') {
      onDiagnose(environmentRequest)
    } else {
      showNotice.info(check.remediation)
    }
  }

  const handleSubmit = async () => {
    setSubmitted(true)
    if (disabled) return
    await onSubmit(captureRequestFromDraft(draft))
  }

  const showError = (key: keyof typeof errors) => submitted && errors[key]
  const interfaceOptions = Array.from(
    new Set([...interfaces, draft.tun_interface, draft.physical_interface]),
  ).filter(Boolean)

  return (
    <Stack spacing={2}>
      <TrafficTracerEnvironmentCard
        report={environment}
        request={diagnosticRequest ?? environmentRequest}
        loading={diagnosing}
        error={diagnosticError}
        onRetry={
          diagnosticsCurrent && onRetryDiagnostics
            ? onRetryDiagnostics
            : () => onDiagnose(environmentRequest)
        }
        onRemediate={handleRemediation}
      />

      {!diagnosticsCurrent && diagnosticRequest && (
        <Alert severity="warning">
          Capture settings changed after the last environment check. Check the
          environment again before starting.
        </Alert>
      )}

      <Paper variant="outlined" sx={{ p: 2 }}>
        <Stack spacing={2}>
          <Box>
            <Typography variant="h6" sx={{ fontSize: 17, fontWeight: 600 }}>
              New capture
            </Typography>
            <Typography variant="body2" color="text.secondary">
              Capture the pre-proxy and post-proxy traffic for one browser
              target.
            </Typography>
          </Box>

          <Box
            sx={{
              display: 'grid',
              gridTemplateColumns: { xs: '1fr', md: '2fr 1fr' },
              gap: 2,
            }}
          >
            <TextField
              label="Target URL"
              value={draft.url}
              error={Boolean(showError('url'))}
              helperText={showError('url') || 'HTTP(S) URL opened by Chrome.'}
              onChange={(event) => {
                const url = event.target.value
                setDraft((current) => ({
                  ...current,
                  url,
                  domain: deriveDomain(url) || current.domain,
                }))
              }}
            />
            <TextField
              label="Domain"
              value={draft.domain}
              error={Boolean(showError('domain'))}
              helperText={showError('domain') || 'Derived from the target URL.'}
              onChange={(event) => update('domain', event.target.value)}
            />
            <TextField
              label="Duration (seconds)"
              type="number"
              value={draft.duration_seconds}
              error={Boolean(showError('duration_seconds'))}
              helperText={showError('duration_seconds')}
              slotProps={{ htmlInput: { min: 1, max: 86_400 } }}
              onChange={(event) =>
                update('duration_seconds', Number(event.target.value))
              }
            />
            <TextField
              select
              label="Network"
              value={draft.network}
              onChange={(event) =>
                update(
                  'network',
                  event.target.value as CaptureFormDraft['network'],
                )
              }
            >
              <MenuItem value="all">TCP + UDP</MenuItem>
              <MenuItem value="tcp">TCP</MenuItem>
              <MenuItem value="udp">UDP</MenuItem>
            </TextField>
            <TextField
              inputRef={tunRef}
              select
              label="TUN interface"
              value={draft.tun_interface}
              error={Boolean(showError('tun_interface'))}
              helperText={showError('tun_interface')}
              onChange={(event) => update('tun_interface', event.target.value)}
            >
              {interfaceOptions.map((name) => (
                <MenuItem key={`tun-${name}`} value={name}>
                  {name}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              inputRef={physicalRef}
              select
              label="Physical interface"
              value={draft.physical_interface}
              error={Boolean(showError('physical_interface'))}
              helperText={showError('physical_interface')}
              onChange={(event) =>
                update('physical_interface', event.target.value)
              }
            >
              {interfaceOptions.map((name) => (
                <MenuItem key={`physical-${name}`} value={name}>
                  {name}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              label="Chrome / Chromium"
              value={draft.chrome_binary}
              error={Boolean(showError('chrome_binary'))}
              helperText={
                showError('chrome_binary') || 'An absolute executable path.'
              }
              onChange={(event) => update('chrome_binary', event.target.value)}
              slotProps={{
                input: {
                  endAdornment: (
                    <InputAdornment position="end">
                      <Button onClick={pickChrome}>Browse</Button>
                    </InputAdornment>
                  ),
                },
              }}
            />
            <TextField
              label="Session output directory"
              value={draft.output_root}
              error={Boolean(showError('output_root'))}
              helperText={
                showError('output_root') ||
                'One subdirectory is created per Session.'
              }
              onChange={(event) => update('output_root', event.target.value)}
              slotProps={{
                input: {
                  endAdornment: (
                    <InputAdornment position="end">
                      <Button onClick={pickOutput}>Browse</Button>
                    </InputAdornment>
                  ),
                },
              }}
            />
          </Box>

          <Stack direction="row" spacing={2} sx={{ flexWrap: 'wrap' }}>
            {(
              [
                ['capture_packets', 'Packet capture'],
                ['collect_cdp', 'Chrome CDP'],
                ['collect_netlog', 'Chrome NetLog'],
                ['analyze_after_capture', 'Analyze automatically'],
                ['headless', 'Headless Chrome'],
              ] as const
            ).map(([key, label]) => (
              <FormControlLabel
                key={key}
                label={label}
                control={
                  <Checkbox
                    checked={draft.options[key]}
                    onChange={(_, checked) => updateOption(key, checked)}
                  />
                }
              />
            ))}
          </Stack>

          {captureLocked && (
            <Alert severity="info">
              A TrafficTracer capture is active. Core and capture settings are
              locked.
            </Alert>
          )}

          <Stack
            direction="row"
            spacing={1}
            sx={{ justifyContent: 'flex-end' }}
          >
            <Button
              variant="outlined"
              startIcon={<SearchRounded />}
              disabled={
                diagnosing || !isAbsolutePlatformPath(draft.output_root)
              }
              onClick={() => {
                setSubmitted(true)
                onDiagnose(environmentRequest)
              }}
            >
              Check environment
            </Button>
            <Button
              variant="contained"
              startIcon={<PlayArrowRounded />}
              disabled={disabled}
              onClick={() => void handleSubmit()}
            >
              Start capture
            </Button>
          </Stack>
        </Stack>
      </Paper>
    </Stack>
  )
}
