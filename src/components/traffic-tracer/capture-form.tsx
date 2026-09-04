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
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useNavigate } from 'react-router'

import {
  getNetworkInterfaces,
  getVergeConfig,
  loadTrafficTracerTargetConfig,
} from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import type {
  CaptureStartRequest,
  BatchStartRequest,
  CompleteEnvironmentReport,
  DiagnosticCheck,
  EnvironmentRequest,
  TargetConfigPreview,
} from '@/types/traffic-tracer'

import {
  applyTargetConfigEntry,
  batchRequestFromDraft,
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

const TARGET_SELECTION_STORAGE_KEY = 'traffictracer.targetSelection.v1'

function restoredTargetIndexes(preview: TargetConfigPreview): Set<number> {
  try {
    const stored = localStorage.getItem(TARGET_SELECTION_STORAGE_KEY)
    if (!stored) throw new Error()
    const selection = JSON.parse(stored) as {
      config_path?: unknown
      config_sha256?: unknown
      indexes?: unknown
    }
    if (
      selection.config_path !== preview.config_path ||
      selection.config_sha256 !== preview.sha256 ||
      !Array.isArray(selection.indexes)
    ) {
      throw new Error()
    }
    const available = new Set(preview.targets.map((target) => target.index))
    const restored = selection.indexes.filter(
      (index): index is number =>
        typeof index === 'number' && available.has(index),
    )
    if (restored.length > 0) return new Set(restored)
  } catch {
    // A changed or invalid config safely falls back to selecting all targets.
  }
  return new Set(preview.targets.map((target) => target.index))
}
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
  onSubmitBatch?: (request: BatchStartRequest) => Promise<unknown> | void
  pipelineEnabled?: boolean
  pipelineCandidateCount?: number
  pipelineRepetitions?: number
  onSelectedTargetCountChange?: (count: number) => void
  onSubmitPipeline?: (request: BatchStartRequest) => Promise<unknown> | void
}

function restoredDraft(): CaptureFormDraft {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (!stored) return defaultCaptureFormDraft
    const parsed = JSON.parse(stored) as Partial<CaptureFormDraft>
    // The workspace root is owned by persisted Verge config, not browser storage.
    delete parsed.output_root
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
  onSubmitBatch,
  pipelineEnabled = false,
  pipelineCandidateCount = 0,
  pipelineRepetitions = 1,
  onSelectedTargetCountChange,
  onSubmitPipeline,
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
  const [selectedTargetIndexes, setSelectedTargetIndexes] = useState<
    Set<number>
  >(() => new Set())

  useEffect(() => {
    onSelectedTargetCountChange?.(selectedTargetIndexes.size)
  }, [onSelectedTargetCountChange, selectedTargetIndexes])

  const tunRef = useRef<HTMLInputElement>(null)
  const physicalRef = useRef<HTMLInputElement>(null)
  const automaticConfigPathRef = useRef<string | null>(null)

  useEffect(() => {
    let disposed = false
    Promise.all([getNetworkInterfaces(), appDataDir(), getVergeConfig()])
      .then(async ([available, dataDir, verge]) => {
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
          output_root:
            current.output_root ||
            verge?.traffic_tracer_output_root ||
            outputRoot,
        }))
      })
      .catch(console.error)
    return () => {
      disposed = true
    }
  }, [])

  useEffect(() => {
    const browserDraft: Partial<CaptureFormDraft> = { ...draft }
    delete browserDraft.output_root
    localStorage.setItem(STORAGE_KEY, JSON.stringify(browserDraft))
  }, [draft])

  useEffect(() => {
    if (!diagnosticError) return
    let disposed = false
    Promise.all([getVergeConfig(), appDataDir()])
      .then(async ([verge, dataDir]) => {
        const defaultRoot = await join(dataDir, 'traffictracer-sessions')
        if (disposed) return
        const acceptedRoot =
          verge?.traffic_tracer_output_root?.trim() || defaultRoot
        setDraft((current) =>
          current.output_root === acceptedRoot
            ? current
            : { ...current, output_root: acceptedRoot },
        )
      })
      .catch(console.error)
    return () => {
      disposed = true
    }
  }, [diagnosticError])

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
    (pipelineEnabled &&
      (draft.target_mode !== 'config' ||
        !targetConfig ||
        pipelineCandidateCount === 0)) ||
    (draft.target_mode === 'config' &&
      Boolean(targetConfig) &&
      selectedTargetIndexes.size === 0) ||
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

  const loadTargetConfig = useCallback(
    async (path: string, preferredIndex?: number | null) => {
      setTargetConfigLoading(true)
      setTargetConfigError('')
      try {
        const preview = await loadTrafficTracerTargetConfig(path)
        const restoredIndexes = restoredTargetIndexes(preview)
        const selected =
          preview.targets.find((target) => target.index === preferredIndex) ??
          preview.targets.find((target) => restoredIndexes.has(target.index)) ??
          preview.targets[0]
        if (!selected) throw new Error('Target configuration has no sites.')
        setTargetConfig(preview)
        setSelectedTargetIndexes(restoredIndexes)
        setDraft((current) =>
          applyTargetConfigEntry(current, preview, selected),
        )
      } catch (error) {
        setTargetConfig(null)
        setTargetConfigError(String(error))
      } finally {
        setTargetConfigLoading(false)
      }
    },
    [],
  )

  useEffect(() => {
    if (draft.target_mode !== 'config' || !draft.config_path.trim()) return
    const path = draft.config_path.trim()
    if (automaticConfigPathRef.current === path) return
    automaticConfigPathRef.current = path
    void loadTargetConfig(path, draft.selected_target_index)
  }, [
    draft.config_path,
    draft.selected_target_index,
    draft.target_mode,
    loadTargetConfig,
  ])

  useEffect(() => {
    if (!targetConfig) return
    localStorage.setItem(
      TARGET_SELECTION_STORAGE_KEY,
      JSON.stringify({
        config_path: targetConfig.config_path,
        config_sha256: targetConfig.sha256,
        indexes: [...selectedTargetIndexes].sort((left, right) => left - right),
      }),
    )
  }, [selectedTargetIndexes, targetConfig])

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
    if (
      pipelineEnabled &&
      targetConfig &&
      selectedTargetIndexes.size > 0 &&
      onSubmitPipeline
    ) {
      await onSubmitPipeline(
        batchRequestFromDraft(draft, targetConfig, selectedTargetIndexes),
      )
      return
    }
    if (
      targetConfig &&
      (selectedTargetIndexes.size > 1 || draft.application_retry_enabled) &&
      onSubmitBatch
    ) {
      await onSubmitBatch(
        batchRequestFromDraft(draft, targetConfig, selectedTargetIndexes),
      )
      return
    }
    if (targetConfig && selectedTargetIndexes.size === 1) {
      const [index] = selectedTargetIndexes
      const target = targetConfig.targets.find((item) => item.index === index)
      if (target) {
        await onSubmit(
          captureRequestFromDraft(
            applyTargetConfigEntry(draft, targetConfig, target),
          ),
        )
        return
      }
    }
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
              onChange={(event) => {
                const targetMode = event.target
                  .value as CaptureFormDraft['target_mode']
                setDraft((current) => ({
                  ...current,
                  target_mode: targetMode,
                  playback: targetMode === 'manual' ? null : current.playback,
                }))
              }}
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
                    {target.playback
                      ? ` — playback ${target.playback.desired_primary_seconds}s/${target.duration_seconds}s`
                      : ''}
                  </MenuItem>
                ))}
              </TextField>
            )}
            {draft.playback && (
              <Alert severity="info" sx={{ gridColumn: '1 / -1' }}>
                YouTube playback: fixed {draft.duration_seconds}s capture;
                target {draft.playback.desired_primary_seconds}s of primary
                video; visible Skip controls are clicked and ad traffic is kept.
              </Alert>
            )}
            {draft.target_mode === 'config' && targetConfig && (
              <Paper variant="outlined" sx={{ p: 1.5, gridColumn: '1 / -1' }}>
                <Stack spacing={0.5}>
                  <Stack
                    direction="row"
                    sx={{
                      alignItems: 'center',
                      justifyContent: 'space-between',
                    }}
                  >
                    <Typography variant="subtitle2">
                      Capture group targets ({selectedTargetIndexes.size}/
                      {targetConfig.targets.length})
                    </Typography>
                    <Button
                      size="small"
                      onClick={() =>
                        setSelectedTargetIndexes(
                          selectedTargetIndexes.size ===
                            targetConfig.targets.length
                            ? new Set()
                            : new Set(
                                targetConfig.targets.map(
                                  (target) => target.index,
                                ),
                              ),
                        )
                      }
                    >
                      {selectedTargetIndexes.size ===
                      targetConfig.targets.length
                        ? 'Clear all'
                        : 'Select all'}
                    </Button>
                  </Stack>
                  <Box
                    sx={{
                      maxHeight: 240,
                      overflowY: 'auto',
                      border: 1,
                      borderColor: 'divider',
                      borderRadius: 1,
                      px: 1,
                    }}
                  >
                    {targetConfig.targets.map((target, position) => (
                      <FormControlLabel
                        key={target.index}
                        sx={{
                          display: 'flex',
                          m: 0,
                          minHeight: 36,
                          '& .MuiFormControlLabel-label': {
                            minWidth: 0,
                            overflow: 'hidden',
                            textOverflow: 'ellipsis',
                            whiteSpace: 'nowrap',
                          },
                        }}
                        title={`${target.page_type} — ${target.domain} — ${target.url}${target.playback ? ` — playback ${target.playback.desired_primary_seconds}s/${target.duration_seconds}s` : ''}`}
                        control={
                          <Checkbox
                            size="small"
                            checked={selectedTargetIndexes.has(target.index)}
                            onChange={(_, checked) =>
                              setSelectedTargetIndexes((current) => {
                                const next = new Set(current)
                                if (checked) next.add(target.index)
                                else next.delete(target.index)
                                return next
                              })
                            }
                          />
                        }
                        label={`${position + 1}. ${target.page_type} — ${target.domain} — ${target.url}${target.playback ? ` — playback ${target.playback.desired_primary_seconds}s/${target.duration_seconds}s` : ''}`}
                      />
                    ))}
                  </Box>
                  <Typography variant="caption" color="text.secondary">
                    Selected targets run sequentially in YAML order. Each
                    capture is cleaned up and analyzed before the next starts.
                  </Typography>
                </Stack>
              </Paper>
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

          {draft.target_mode === 'config' && targetConfig && (
            <Box>
              <FormControlLabel
                label="Retry classified activity failure once"
                control={
                  <Checkbox
                    checked={draft.application_retry_enabled}
                    onChange={(_, checked) =>
                      update('application_retry_enabled', checked)
                    }
                  />
                }
              />
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ display: 'block', ml: 4 }}
              >
                Enabled by default: use at most one fresh Chrome and Session
                attempt for explicit transient page-load, critical-resource, or
                playback outcomes. Deterministic HTTP 4xx, capture, analysis,
                protocol, and unclassified failures are never retried.
              </Typography>
            </Box>
          )}

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
                    checked={
                      key === 'analyze_after_capture' &&
                      (selectedTargetIndexes.size > 1 ||
                        draft.application_retry_enabled)
                        ? true
                        : draft.options[key]
                    }
                    disabled={
                      key === 'analyze_after_capture' &&
                      (selectedTargetIndexes.size > 1 ||
                        draft.application_retry_enabled)
                    }
                    onChange={(_, checked) => updateOption(key, checked)}
                  />
                }
              />
            ))}
          </Stack>

          <TextField
            select
            size="small"
            label={t('settings.trafficTracer.capture.fields.cachePolicy')}
            value={draft.options.cache_mode}
            helperText={
              draft.options.cache_mode === 'cold'
                ? t('settings.trafficTracer.capture.hints.cacheCold')
                : t('settings.trafficTracer.capture.hints.cacheWarm')
            }
            onChange={(event) =>
              setDraft((current) => ({
                ...current,
                options: {
                  ...current.options,
                  cache_mode: event.target.value as 'cold' | 'warm',
                },
              }))
            }
          >
            <MenuItem value="cold">
              {t('settings.trafficTracer.capture.values.cacheCold')}
            </MenuItem>
            <MenuItem value="warm">
              {t('settings.trafficTracer.capture.values.cacheWarm')}
            </MenuItem>
          </TextField>

          <TextField
            select
            size="small"
            label="Proxy protocol invariant"
            value={draft.options.proxy_protocol_mode}
            helperText={
              draft.options.proxy_protocol_mode === 'strict_single'
                ? 'Block capture when selected proxy leaves use multiple protocols.'
                : 'Record mixed protocols diagnostically without blocking capture.'
            }
            onChange={(event) =>
              setDraft((current) => ({
                ...current,
                options: {
                  ...current.options,
                  proxy_protocol_mode: event.target.value as
                    | 'strict_single'
                    | 'observe',
                },
              }))
            }
          >
            <MenuItem value="strict_single">Strict single protocol</MenuItem>
            <MenuItem value="observe">Observe only</MenuItem>
          </TextField>

          <TextField
            size="small"
            label="Expected proxy protocol (optional)"
            value={draft.options.expected_proxy_protocol}
            placeholder="hysteria2"
            helperText={
              draft.options.expected_proxy_protocol
                ? `Require ${draft.options.expected_proxy_protocol} for proxied flows; DIRECT and REJECT are exempt.`
                : draft.options.proxy_selection_group
                  ? 'Leave empty to freeze the selected group leaf protocol automatically.'
                  : 'Leave empty to validate that the runtime trace uses at most one proxy protocol.'
            }
            onChange={(event) =>
              setDraft((current) => ({
                ...current,
                options: {
                  ...current.options,
                  expected_proxy_protocol: event.target.value
                    .trim()
                    .toLowerCase(),
                },
              }))
            }
          />

          <TextField
            size="small"
            label="Protocol selection group (optional)"
            value={draft.options.proxy_selection_group}
            placeholder="Proxy"
            helperText="Only this selected chain is checked before capture; runtime trace remains authoritative."
            onChange={(event) =>
              setDraft((current) => ({
                ...current,
                options: {
                  ...current.options,
                  proxy_selection_group: event.target.value,
                },
              }))
            }
          />

          <TextField
            select
            size="small"
            label={t('settings.trafficTracer.capture.fields.analysisStorage')}
            value={draft.options.pcap_split_mode}
            disabled={
              !draft.options.capture_packets ||
              !draft.options.analyze_after_capture
            }
            helperText={
              draft.options.pcap_split_mode === 'none'
                ? t('settings.trafficTracer.capture.hints.analysisStandard')
                : t('settings.trafficTracer.capture.hints.analysisFull')
            }
            onChange={(event) =>
              setDraft((current) => ({
                ...current,
                options: {
                  ...current.options,
                  pcap_split_mode: event.target.value as
                    | 'none'
                    | 'unique_connections',
                },
              }))
            }
          >
            <MenuItem value="none">
              {t('settings.trafficTracer.capture.values.analysisStandard')}
            </MenuItem>
            <MenuItem value="unique_connections">
              {t('settings.trafficTracer.capture.values.analysisFull')}
            </MenuItem>
          </TextField>

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
              {pipelineEnabled
                ? `Start pipeline (${pipelineCandidateCount} nodes × ${pipelineRepetitions})`
                : t('settings.trafficTracer.common.actions.startCapture')}
            </Button>
          </Stack>
        </Stack>
      </Paper>
    </Stack>
  )
}
