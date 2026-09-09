/* eslint-disable react-refresh/only-export-components -- Isolated native test entry. */
import { QueryClientProvider } from '@tanstack/react-query'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import i18next from 'i18next'
import { useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { initReactI18next } from 'react-i18next'
import { MemoryRouter } from 'react-router'

import { trafficTracerEnvironmentKey } from '../../src/hooks/use-traffic-tracer-worker'
import resources from '../../src/locales/en'
import TrafficTracerPage from '../../src/pages/traffic-tracer'
import { queryClient as client } from '../../src/services/query-client'

import { subscriptions } from './event-boundary'
import { counters, environment, request, scenario } from './full-fixtures'

const stats = { received: 0, latest: 0, remounts: 0, errors: 0, rendered: 0 }
void import('../../src/utils/traffic-tracer-notification-ack')
  .then(async ({ installNotificationAcknowledgement }) => {
    await installNotificationAcknowledgement()
  })
  .catch(() => {
    stats.errors++
  })
void import('../../src/utils/traffic-tracer-native-recovery')
  .then(async ({ installNativeDesktopRecovery }) => {
    await installNativeDesktopRecovery()
  })
  .catch(() => {
    stats.errors++
  })
void import('../../src/utils/traffic-tracer-progress-ack')
  .then(async ({ installProgressAcknowledgement }) => {
    await installProgressAcknowledgement()
  })
  .catch(() => {
    stats.errors++
  })
let desktopRecoveries = 0
window.addEventListener('traffictracer-desktop-recovered', () => {
  desktopRecoveries++
})
// Passive diagnostics: do not replace the original heartbeat/health gate.
const diagnostic = {
  driverStage: 'startup',
  driverAt: performance.now(),
  frameAt: performance.now(),
  frames: 0,
}
const visibilityChanges: Array<{ at: number; state: string }> = []
document.addEventListener('visibilitychange', () => {
  visibilityChanges.push({
    at: performance.now(),
    state: document.visibilityState,
  })
  if (visibilityChanges.length > 16) visibilityChanges.shift()
})
const trackFrame = () => {
  diagnostic.frameAt = performance.now()
  diagnostic.frames++
  requestAnimationFrame(trackFrame)
}
requestAnimationFrame(trackFrame)
let probePending = false
let lastProbe = 0
const page = {
  mode: 'full',
  mounts: 0,
  detailDialogs: 0,
  paginationClicks: 0,
  historySelections: 0,
  historyLabelFound: false,
  historyComboFound: false,
  historyControlNames: [] as string[],
  batchSelections: 0,
  pipelineSelections: 0,
  errorBoundary: false,
  scenario,
  visibleStatusErrors: 0,
  clearedStatusErrors: 0,
  cleanupChecks: 0,
  cleanupFailures: 0,
  analysisRowsPeak: 0,
}
window.addEventListener('error', () => stats.errors++)
window.addEventListener('unhandledrejection', () => stats.errors++)
await i18next
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: resources } } })
// This WebView has a fresh, isolated data directory; never load the app's stores.
localStorage.setItem(
  'traffictracer.environmentRequest.v1',
  JSON.stringify(request),
)
localStorage.setItem('traffictracer.activeJobId', 'isolated-soak')
localStorage.setItem(
  `traffictracer.activeBatchId:${encodeURIComponent(request.output_root)}`,
  'fixture-batch-0',
)
client.setQueryData(trafficTracerEnvironmentKey(request), environment)
await listen<{ sequence: number }>(
  'traffictracer://job-progress',
  ({ payload }) => {
    stats.received++
    stats.latest = payload.sequence
    const now = performance.now()
    if (!probePending && now - lastProbe >= 1000) {
      probePending = true
      lastProbe = now
      void invoke('soak_probe', {
        data: {
          received: stats.received,
          latest: stats.latest,
          visibility: document.visibilityState,
          desktopRecoveries,
          focused: document.hasFocus(),
          visibilityChanges: [...visibilityChanges],
          driverStage: diagnostic.driverStage,
          driverAgeMs: now - diagnostic.driverAt,
          frameAgeMs: now - diagnostic.frameAt,
          frames: diagnostic.frames,
        },
      })
        .catch(() => {
          stats.errors++
        })
        .finally(() => {
          probePending = false
        })
    }
  },
)

function App() {
  const [visible, setVisible] = useState(true)
  useEffect(() => {
    page.mounts++
    let tick = 0
    let pending = false
    let hadStatusError = false
    const timer = window.setInterval(async () => {
      if (pending) return
      pending = true
      diagnostic.driverStage = 'actions'
      diagnostic.driverAt = performance.now()
      if (tick === 19 && scenario === 'driver_stall') {
        diagnostic.driverStage = 'injected_driver_stall'
        return // Deliberately retain pending while native events continue.
      }
      if (tick === 19 && scenario === 'js_stall') {
        diagnostic.driverStage = 'injected_js_stall'
        const until = performance.now() + 12000
        while (performance.now() < until) {
          /* Isolated negative test only. */
        }
      }
      try {
        tick++
        const text =
          document.querySelector('[data-testid="traffic-tracer-job-progress"]')
            ?.textContent ?? ''
        const sequence = text.match(/Synthetic analysis progress (\d+)/)?.[1]
        if (sequence) stats.rendered = Number(sequence)
        // The production boundary catches errors, so count its fallback explicitly.
        page.errorBoundary ||=
          document.body.textContent?.includes('Something went wrong:(') ?? false
        const statusError =
          document.body.textContent?.includes('SOAK_STATUS_UNAVAILABLE') ??
          false
        if (statusError) {
          page.visibleStatusErrors++
          hadStatusError = true
        } else if (
          document.querySelector(
            '[data-testid="traffic-tracer-job-progress"]',
          ) &&
          hadStatusError
        ) {
          page.clearedStatusErrors++
          hadStatusError = false
        }
        const phase = tick % 12
        if (scenario === 'slow_recovery' && phase === 6) {
          const resume = [...document.querySelectorAll('button')].find(
            (button) => button.textContent === 'Resume pipeline',
          )
          if (resume && !resume.disabled) resume.click()
        }
        if (phase === 3) {
          const card = document.querySelector(
            '[data-testid^="traffic-tracer-session-fixture-"]',
          )
          const button = [...(card?.querySelectorAll('button') ?? [])].find(
            (item) =>
              item.textContent ===
              resources.settings.trafficTracer.sessions.details,
          )
          button?.click()
        }
        if (phase === 5) {
          const dialog = document.querySelector('[role="dialog"]')
          page.analysisRowsPeak = Math.max(
            page.analysisRowsPeak,
            dialog?.querySelectorAll('tbody tr').length ?? 0,
          )
          if (dialog?.textContent?.includes('Connection-centric analysis'))
            page.detailDialogs++
          const close = [...(dialog?.querySelectorAll('button') ?? [])].find(
            (item) =>
              item.textContent ===
              resources.settings.trafficTracer.common.actions.close,
          )
          close?.click()
        }
        if (phase === 7) {
          const next = document.querySelector<HTMLButtonElement>(
            'button[aria-label="Go to next page"]',
          )
          if (next && !next.disabled) {
            next.click()
            page.paginationClicks++
          }
        }
        if (phase === 8) {
          const historyLabel =
            Math.floor(tick / 12) % 2 === 0
              ? 'Capture Group history'
              : 'Profile / node pipeline history'
          const controls = [...document.querySelectorAll('[role="combobox"]')]
          const controlName = (element: Element) =>
            (element.getAttribute('aria-labelledby') ?? '')
              .split(/\s+/)
              .map((id) => document.getElementById(id)?.textContent ?? '')
              .join(' ')
          page.historyControlNames = controls.map(controlName).slice(0, 10)
          const combo = controls.find((element) =>
            controlName(element).includes(historyLabel),
          )
          page.historyLabelFound = Boolean(combo)
          page.historyComboFound = Boolean(combo)
          if (combo)
            combo.dispatchEvent(
              new MouseEvent('mousedown', { bubbles: true, button: 0 }),
            )
        }
        if (phase === 9) {
          const options =
            document.querySelectorAll<HTMLElement>('[role="option"]')
          const option = options.item(
            Math.floor(tick / 12) % Math.max(1, options.length),
          )
          if (option) {
            option.click()
            page.historySelections++
            if (Math.floor(tick / 12) % 2 === 0) page.batchSelections++
            else page.pipelineSelections++
          }
        }
        if (phase === 10) setVisible(false)
        if (phase === 11) {
          page.cleanupChecks++
          // The independent harness listener is the only subscription left.
          // Diagnostics, progress ACK, and native recovery are window-owned.
          if (subscriptions.active !== 4 || subscriptions.pending !== 0)
            page.cleanupFailures++
          setVisible(true)
          stats.remounts++
          page.mounts++
        }
        diagnostic.driverStage = 'heartbeat_await'
        diagnostic.driverAt = performance.now()
        await invoke('soak_heartbeat', {
          ...stats,
          errors:
            stats.errors +
            counters.denied +
            Number(page.errorBoundary) +
            page.cleanupFailures,
          pageMetrics: {
            ...page,
            ...counters,
            queries: client.getQueryCache().getAll().length,
            subscriptions: { ...subscriptions },
            analysisQueries: client
              .getQueryCache()
              .getAll()
              .filter((query) => query.queryKey[1] === 'analysis').length,
          },
        })
        diagnostic.driverStage = 'idle'
        diagnostic.driverAt = performance.now()
      } finally {
        pending = false
      }
    }, 1000)
    return () => clearInterval(timer)
  }, [])
  return (
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <header>
          Isolated full TrafficTracer page — synthetic backend only
        </header>
        <div style={{ height: 650, overflow: 'auto' }}>
          {visible ? <TrafficTracerPage /> : <p>Simulated navigation away</p>}
        </div>
      </MemoryRouter>
    </QueryClientProvider>
  )
}
createRoot(document.getElementById('root')!).render(<App />)
