// Isolated entry: never import production main, router, providers or services.
/* eslint-disable react-refresh/only-export-components -- Standalone production-built test entry, no HMR. */
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import i18next from 'i18next'
import { useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { initReactI18next } from 'react-i18next'

import { TrafficTracerJobProgress } from '../../src/components/traffic-tracer/job-progress'
import type {
  JobProgressEvent,
  JobSnapshot,
} from '../../src/types/traffic-tracer'

const stats = { received: 0, latest: 0, remounts: 0, errors: 0, rendered: 0 }
void import('../../src/utils/traffic-tracer-progress-ack')
  .then(async ({ installProgressAcknowledgement }) => {
    await installProgressAcknowledgement()
  })
  .catch(() => {
    stats.errors++
  })
const recent: JobProgressEvent[] = []
window.addEventListener('error', () => stats.errors++)
window.addEventListener('unhandledrejection', () => stats.errors++)
await i18next.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
  fallbackLng: 'en',
})

function ProgressView() {
  const [sequence, setSequence] = useState(stats.latest)
  useEffect(() => {
    let disposed = false
    const subscription = listen<JobProgressEvent & { sequence: number }>(
      'traffictracer://job-progress',
      ({ payload }) => {
        if (disposed) return
        stats.received++
        stats.latest = payload.sequence
        recent.push({ ...payload, timestamp: new Date().toISOString() })
        if (recent.length > 100) recent.shift()
        setSequence(payload.sequence)
      },
    )
    return () => {
      disposed = true
      void subscription
        .then((unlisten) => unlisten())
        .catch(() => stats.errors++)
    }
  }, [])
  useEffect(() => {
    stats.rendered = sequence
  }, [sequence])
  const job: JobSnapshot = {
    job_id: 'isolated-soak',
    kind: 'analysis',
    state: 'analyzing',
    stage: 'analyzing',
    progress: (sequence % 1000) / 1000,
    message: `Native event ${sequence}`,
    cancel_requested: false,
  }
  return (
    <TrafficTracerJobProgress
      job={job}
      events={[...recent]}
      onCancel={() => {}}
    />
  )
}

function App() {
  const [generation, setGeneration] = useState(0)
  useEffect(() => {
    let inFlight = false
    const heartbeat = window.setInterval(async () => {
      if (inFlight) return
      inFlight = true
      try {
        await invoke('soak_heartbeat', stats)
      } catch {
        stats.errors++
      } finally {
        inFlight = false
      }
    }, 1000)
    const remount = window.setInterval(() => {
      stats.remounts++
      setGeneration(stats.remounts)
    }, 5000)
    return () => {
      clearInterval(heartbeat)
      clearInterval(remount)
    }
  }, [])
  return (
    <main>
      <h1>Isolated React progress soak</h1>
      <p>
        No proxy, service, subscription or capture initialization. Real progress
        component; synthetic events.
      </p>
      <div style={{ height: 360, overflow: 'auto' }}>
        <ProgressView key={generation} />
      </div>
    </main>
  )
}
createRoot(document.getElementById('root')!).render(<App />)
