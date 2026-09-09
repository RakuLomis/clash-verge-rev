import type {
  CompleteEnvironmentReport,
  EnvironmentRequest,
  SessionScope,
} from '../../src/types/traffic-tracer'

declare const __TT_SOAK_SCENARIO__: string
export const scenario = __TT_SOAK_SCENARIO__
const pipelineStates = new Map<string, string>()
const pause = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

export const counters = {
  calls: 0,
  inFlight: 0,
  peakInFlight: 0,
  denied: 0,
  details: 0,
  deniedNames: [] as string[],
  delayedCalls: 0,
  statusReads: 0,
  statusFailures: 0,
  resumeCalls: 0,
  largeIndexReads: 0,
}
export const request: EnvironmentRequest = {
  output_root: '/isolated-fixture',
  tun_interface: 'fixture-tun',
  physical_interface: 'fixture-eth',
  chrome_binary: '/fixture/chrome',
}
export const environment: CompleteEnvironmentReport = {
  level: 'ready',
  ok: true,
  checks: [],
  integration: {
    current_core: 'verge-mihomo-traffictracer',
    tun_enabled: true,
    service_available: true,
    configured_tun_device: 'fixture-tun',
    automatic_tun_device: 'fixture-tun',
    capture_tun_interface: 'fixture-tun',
    worker: { state: 'ready' },
  },
}
const scope: SessionScope = {
  scope_id: 'fixture-group',
  display_name: 'Synthetic capture group',
  directory: '/isolated-fixture/group',
  kind: 'capture_group',
  created_at: '2026-09-09T00:00:00Z',
  exists: true,
}
const stamp = '2026-09-09T00:00:00Z'
const sessions = Array.from({ length: 12 }, (_, index) => ({
  schema_version: 1,
  session_id: `fixture-${index}`,
  job_id: 'isolated-soak',
  state: 'completed',
  created_at: stamp,
  updated_at: stamp,
  session_dir: `/isolated-fixture/group/${index}`,
  target: { url: `https://example.test/page-${index}`, domain: 'example.test' },
  artifact_count: 0,
  warning_count: 0,
  quality_state: 'passed',
  capture_global_quality_state: 'passed',
  coverage: null,
}))
const version = { version: 'synthetic', commit: '' }
function batch(index: number) {
  return {
    schema_version: 1,
    batch_id: `fixture-batch-${index}`,
    state: 'completed',
    stage: 'finished',
    created_at: stamp,
    updated_at: stamp,
    output_root: request.output_root,
    config: { path: '/fixture/sites.yaml', sha256: 'synthetic' },
    targets: sessions.map((item, targetIndex) => ({
      index: targetIndex,
      ...item.target,
      duration_seconds: 10,
      network: 'all',
      run_label: 'synthetic',
      wait_load_timeout: 5,
      page_type: 'static',
    })),
    children: sessions.map((item, targetIndex) => ({
      target_index: targetIndex,
      state: 'completed',
      session_id: item.session_id,
      error: null,
    })),
    current_index: null,
    fail_fast: false,
    cancel_requested: false,
    resume: { attempt: 0, next_index: sessions.length, resumed_at: null },
  }
}
export async function command(name: string, args: unknown[]): Promise<unknown> {
  counters.calls++
  counters.inFlight++
  counters.peakInFlight = Math.max(counters.peakInFlight, counters.inFlight)
  try {
    if (scenario === 'slow_recovery' && name === 'readTrafficTracerAnalysis') {
      counters.delayedCalls++
      // Outlast the details window; this must not retain abandoned queries.
      await pause(counters.details % 2 === 0 ? 100 : 3000)
    }
    // Only explicit read-only synthetic responses are allowed. No fallback IPC.
    switch (name) {
      case 'getNetworkInterfaces':
        return ['fixture-tun', 'fixture-eth']
      case 'getVergeConfig':
        return { traffic_tracer_output_root: request.output_root }
      case 'getTrafficTracerEnvironment':
        return environment
      case 'getTrafficTracerCaptureLock':
        return { locked: false }
      case 'getTrafficTracerJob':
        return {
          job_id: 'isolated-soak',
          kind: 'analysis',
          state: 'analyzing',
          stage: 'analyzing',
          progress: 0.5,
          message: 'Synthetic workload',
          cancel_requested: false,
        }
      case 'listTrafficTracerPipelines':
        return [0, 1].map((index) => ({
          pipeline_id: `fixture-pipeline-${index}`,
          output_root: `/isolated-fixture/pipeline-${index}`,
          updated_at: stamp,
          state: 'completed',
          completed_runs: 0,
          total_runs: 0,
          candidate_count: 1,
          repetitions_per_candidate: 1,
        }))
      case 'resumeTrafficTracerPipeline': {
        if (scenario !== 'slow_recovery')
          throw new Error(
            'Resume only exists in the synthetic recovery scenario',
          )
        counters.resumeCalls++
        await pause(1500)
        pipelineStates.set(String(args[0]), 'completed')
        return await command('getTrafficTracerPipeline', args)
      }
      case 'getTrafficTracerPipeline': {
        const key = String(args[0])
        const state = pipelineStates.get(key) ?? 'completed'
        counters.statusReads++
        const fail =
          scenario === 'slow_recovery' && counters.statusReads % 6 === 2
        if (scenario === 'slow_recovery') {
          counters.delayedCalls++
          await pause(2500)
          if (fail) {
            counters.statusFailures++
            pipelineStates.set(key, 'interrupted')
            throw new Error(
              'SOAK_STATUS_UNAVAILABLE: synthetic delayed status failure',
            )
          }
        }
        return {
          schema_version: 7,
          pipeline_id: String(args[0]).split('/').at(-1),
          output_root: args[0],
          state,
          stage: 'finished',
          created_at: stamp,
          updated_at: stamp,
          config: { path: '/fixture/sites.yaml', sha256: 'synthetic' },
          targets: [],
          execution: {},
          policy: {
            continue_on_run_failure: true,
            restore_original_state: true,
          },
          repetitions_per_candidate: 1,
          current_run_index: null,
          runs: [],
          schedule: {
            mode: 'candidate_major',
            candidate_order_policy: 'fixed',
            random_seed: null,
            algorithm_version: 1,
            repetition_candidate_orders: [],
          },
          restore: {
            profile_uid: null,
            selections: [],
            checks: [],
            state: 'not_required',
            error: null,
          },
        }
      }
      case 'listTrafficTracerBatches':
        return { batches: [batch(0), batch(1)], corrupt: [] }
      case 'getTrafficTracerBatch':
        return {
          batch: batch(args[0] === 'fixture-batch-1' ? 1 : 0),
          job: null,
        }
      case 'resolveTrafficTracerSessionScope':
        return scope
      case 'previewTrafficTracerPacketSplit':
        return {
          scope,
          total: sessions.length,
          counts: { complete_empty: sessions.length },
          missing_only: 0,
          repair_incomplete: 0,
          sessions: sessions.map((item) => ({
            session_id: item.session_id,
            url: item.target.url,
            status: 'complete_empty',
            reason: 'Synthetic empty capture',
            connection_count: 0,
            runnable_missing: false,
            runnable_repair: false,
          })),
          corrupt: [],
        }
      case 'listTrafficTracerScopedSessions': {
        const offset = Number(args[1] ?? 0)
        const limit = Number(args[2] ?? 8)
        return {
          scope,
          sessions: sessions.slice(offset, offset + limit),
          corrupt: [],
          offset,
          limit,
          total: sessions.length,
          has_more: offset + limit < sessions.length,
        }
      }
      case 'listTrafficTracerSessions':
        return {
          sessions,
          corrupt: [],
          offset: 0,
          limit: 20,
          total: sessions.length,
          has_more: false,
        }
      case 'getTrafficTracerSession': {
        const session = sessions.find((item) => item.session_id === args[0])
        if (!session) throw new Error('Unknown synthetic Session')
        counters.details++
        return {
          ...session,
          warnings: [],
          artifacts: [],
          error: null,
          component_versions: {
            worker_api: 2,
            traffictracer: version,
            mihomo: version,
            clash_verge_rev: version,
          },
        }
      }
      case 'readTrafficTracerAnalysis': {
        if (scenario === 'slow_recovery' && args[1] !== 'coverage_summary') {
          counters.largeIndexReads++
          const items = Array.from({ length: 1000 }, (_, index) => {
            const url = `https://example.test/large/${index}`
            const evidence = {
              status: 'matched',
              method: 'synthetic',
              confidence: 1,
              evidence: [],
              candidates: [],
            }
            return args[1] === 'request_index'
              ? {
                  request_id: `request-${index}`,
                  url,
                  resource_type: 'Document',
                  relation: 'main',
                  connection_id: `connection-${index}`,
                  candidate_connection_ids: [],
                  attribution: evidence,
                }
              : {
                  connection_id: `connection-${index}`,
                  protocol: 'tcp',
                  pre_flow: {
                    network: 'tcp',
                    src_ip: '192.0.2.1',
                    src_port: 1234,
                    dst_ip: '198.51.100.1',
                    dst_port: 443,
                    complete: true,
                    source: 'synthetic',
                    scope: 'pre_proxy',
                    shared: false,
                  },
                  post_flow: null,
                  shared: false,
                  request_ids: [`request-${index}`],
                  urls: [url],
                  primary_url: url,
                  match: evidence,
                }
          })
          return { analysis_generation_id: 'synthetic', items }
        }
        return args[1] === 'coverage_summary'
          ? null
          : { schema_version: 1, items: [] }
      }
      default:
        counters.denied++
        if (
          !counters.deniedNames.includes(name) &&
          counters.deniedNames.length < 10
        )
          counters.deniedNames.push(name)
        throw new Error(`ISOLATED_COMMAND_DENIED: ${name}`)
    }
  } finally {
    counters.inFlight--
  }
}
