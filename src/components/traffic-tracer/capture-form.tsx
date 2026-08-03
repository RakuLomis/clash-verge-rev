import {
  PlayArrowRounded,
  RefreshRounded,
  SearchRounded,
} from '@mui/icons-material'
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
import { useTranslation } from 'react-i18next'
import { useNavigate } from 'react-router'

import {
  getNetworkInterfaces,
  loadTrafficTracerTargetConfig,
} from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import type {
  CaptureStartRequest,
  CompleteEnvironmentReport,
  DiagnosticCheck,
  EnvironmentRequest,
  TargetConfigPreview,
} from '@/types/traffic-tracer'

import {
  applyTargetConfigEntry,
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
  const { t } = useTranslation()
  const navigate = useNavigate()
  const [draft, setDraft] = useState(restoredDraft)
  const [interfaces, setInterfaces] = useState<string[]>([])
  const [submitted, setSubmitted] = useState(false)
  const [targetConfig, setTargetConfig] = useState<TargetConfigPreview | null>(
    null,
  )
  const [targetConfigError, setTargetConfigError] = useState('')
  const [targetConfigLoading, setTargetConfigLoading] = useState(false)
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
      title: t('settings.trafficTracer.capture.selectChrome'),
    })
    if (selected) update('chrome_binary', String(selected))
  }

  const pickOutput = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t('settings.trafficTracer.capture.selectOutput'),
    })
    if (selected) update('output_root', String(selected))
  }

  const loadTargetConfig = async (
    path: string,
    preferredIndex?: number | null,
  ) => {
    setTargetConfigLoading(true)
    setTargetConfigError('')
    try {
      const preview = await loadTrafficTracerTargetConfig(path)
      const selected =
        preview.targets.find((target) => target.index === preferredIndex) ??
        preview.targets[0]
      if (!selected) throw new Error('Target configuration has no sites.')
      setTargetConfig(preview)
      setDraft((current) => applyTargetConfigEntry(current, preview, selected))
    } catch (error) {
      setTargetConfig(null)
      setTargetConfigError(String(error))
    } finally {
      setTargetConfigLoading(false)
    }
  }

  const pickTargetConfig = async () => {
    const selected = await open({
      directory: false,
      multiple: false,
      filters: [{ name: 'YAML', extensions: ['yaml', 'yml'] }],
      title: t('settings.trafficTracer.capture.selectTargetConfig'),
    })
    if (selected) await loadTargetConfig(String(selected))
  }

  const selectTarget = (index: number) => {
    const target = targetConfig?.targets.find((item) => item.index === index)
    if (!target || !targetConfig) return
    setDraft((current) => applyTargetConfigEntry(current, targetConfig, target))
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

  const showError = (key: keyof typeof errors) => {
    const code = submitted ? errors[key] : undefined
    return code ? t(`settings.trafficTracer.validation.${code}`) : undefined
  }
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
          {t('settings.trafficTracer.environment.changed')}
        </Alert>
      )}

      <Paper variant="outlined" sx={{ p: 2 }}>
        <Stack spacing={2}>
          <Box>
            <Typography variant="h6" sx={{ fontSize: 17, fontWeight: 600 }}>
              {t('settings.trafficTracer.capture.title')}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              {t('settings.trafficTracer.capture.description')}
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
              select
              label={t('settings.trafficTracer.capture.fields.targetSource')}
              value={draft.target_mode}
              onChange={(event) =>
                update(
                  'target_mode',
                  event.target.value as CaptureFormDraft['target_mode'],
                )
              }
            >
              <MenuItem value="manual">
                {t('settings.trafficTracer.capture.targetSources.manual')}
              </MenuItem>
              <MenuItem value="config">
                {t('settings.trafficTracer.capture.targetSources.config')}
              </MenuItem>
            </TextField>
            {draft.target_mode === 'config' && (
              <TextField
                label={t('settings.trafficTracer.capture.fields.configFile')}
                value={draft.config_path}
                error={Boolean(targetConfigError || showError('config_path'))}
                helperText={
                  targetConfigError ||
                  showError('config_path') ||
                  t('settings.trafficTracer.capture.hints.configFile')
                }
                slotProps={{
                  input: {
                    readOnly: true,
                    endAdornment: (
                      <InputAdornment position="end">
                        {draft.config_path && (
                          <Button
                            disabled={targetConfigLoading}
                            onClick={() =>
                              void loadTargetConfig(
                                draft.config_path,
                                draft.selected_target_index,
                              )
                            }
                          >
                            <RefreshRounded fontSize="small" />
                          </Button>
                        )}
                        <Button
                          disabled={targetConfigLoading}
                          onClick={() => void pickTargetConfig()}
                        >
                          {t('settings.trafficTracer.common.actions.browse')}
                        </Button>
                      </InputAdornment>
                    ),
                  },
                }}
              />
            )}
            {draft.target_mode === 'config' && targetConfig && (
              <TextField
                select
                label={t('settings.trafficTracer.capture.fields.configTarget')}
                value={draft.selected_target_index ?? ''}
                onChange={(event) => selectTarget(Number(event.target.value))}
              >
                {targetConfig.targets.map((target) => (
                  <MenuItem key={target.index} value={target.index}>
                    {target.domain} — {target.url}
                  </MenuItem>
                ))}
              </TextField>
            )}
            <TextField
              label={t('settings.trafficTracer.capture.fields.url')}
              value={draft.url}
              error={Boolean(showError('url'))}
              helperText={
                showError('url') ||
                t('settings.trafficTracer.capture.hints.url')
              }
              onChange={(event) => {
                const url = event.target.value
                setDraft((current) => ({
                  ...current,
                  url,
                  domain: deriveDomain(url) || current.domain,
                }))
              }}
              slotProps={{
                input: { readOnly: draft.target_mode === 'config' },
              }}
            />
            <TextField
              label={t('settings.trafficTracer.capture.fields.domain')}
              value={draft.domain}
              error={Boolean(showError('domain'))}
              helperText={
                showError('domain') ||
                t('settings.trafficTracer.capture.hints.domain')
              }
              onChange={(event) => update('domain', event.target.value)}
              slotProps={{
                input: { readOnly: draft.target_mode === 'config' },
              }}
            />
            <TextField
              label={t('settings.trafficTracer.capture.fields.duration')}
              type="number"
              value={draft.duration_seconds}
              error={Boolean(showError('duration_seconds'))}
              helperText={showError('duration_seconds')}
              onChange={(event) =>
                update('duration_seconds', Number(event.target.value))
              }
              slotProps={{
                htmlInput: {
                  min: 1,
                  max: 86_400,
                  readOnly: draft.target_mode === 'config',
                },
              }}
            />
            <TextField
              select
              label={t('settings.trafficTracer.capture.fields.network')}
              value={draft.network}
              onChange={(event) =>
                update(
                  'network',
                  event.target.value as CaptureFormDraft['network'],
                )
              }
              disabled={draft.target_mode === 'config'}
            >
              <MenuItem value="all">TCP + UDP</MenuItem>
              <MenuItem value="tcp">TCP</MenuItem>
              <MenuItem value="udp">UDP</MenuItem>
            </TextField>
            <TextField
              inputRef={tunRef}
              select
              label={t('settings.trafficTracer.capture.fields.tunInterface')}
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
              label={t(
                'settings.trafficTracer.capture.fields.physicalInterface',
              )}
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
              label={t('settings.trafficTracer.capture.fields.chrome')}
              value={draft.chrome_binary}
              error={Boolean(showError('chrome_binary'))}
              helperText={
                showError('chrome_binary') ||
                t('settings.trafficTracer.capture.hints.chrome')
              }
              onChange={(event) => update('chrome_binary', event.target.value)}
              slotProps={{
                input: {
                  endAdornment: (
                    <InputAdornment position="end">
                      <Button onClick={pickChrome}>
                        {t('settings.trafficTracer.common.actions.browse')}
                      </Button>
                    </InputAdornment>
                  ),
                },
              }}
            />
            <TextField
              label={t('settings.trafficTracer.capture.fields.output')}
              value={draft.output_root}
              error={Boolean(showError('output_root'))}
              helperText={
                showError('output_root') ||
                t('settings.trafficTracer.capture.hints.output')
              }
              onChange={(event) => update('output_root', event.target.value)}
              slotProps={{
                input: {
                  endAdornment: (
                    <InputAdornment position="end">
                      <Button onClick={pickOutput}>
                        {t('settings.trafficTracer.common.actions.browse')}
                      </Button>
                    </InputAdornment>
                  ),
                },
              }}
            />
          </Box>

          {targetConfig?.warnings.map((warning) => (
            <Alert key={warning} severity="warning">
              {warning}
            </Alert>
          ))}

          <Stack direction="row" spacing={2} sx={{ flexWrap: 'wrap' }}>
            {(
              [
                ['capture_packets', 'packets'],
                ['collect_cdp', 'cdp'],
                ['collect_netlog', 'netlog'],
                ['analyze_after_capture', 'analyze'],
                ['headless', 'headless'],
              ] as const
            ).map(([key, label]) => (
              <FormControlLabel
                key={key}
                label={t(`settings.trafficTracer.capture.options.${label}`)}
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
              {t('settings.trafficTracer.locks.captureActive')}{' '}
              {t('settings.trafficTracer.capture.locked')}
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
              {t('settings.trafficTracer.common.actions.checkEnvironment')}
            </Button>
            <Button
              variant="contained"
              startIcon={<PlayArrowRounded />}
              disabled={disabled}
              onClick={() => void handleSubmit()}
            >
              {t('settings.trafficTracer.common.actions.startCapture')}
            </Button>
          </Stack>
        </Stack>
      </Paper>
    </Stack>
  )
}
