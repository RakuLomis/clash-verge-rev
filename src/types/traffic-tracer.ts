export type WorkerManagerState =
  | { state: 'stopped' | 'starting' | 'ready' | 'busy' }
  | { state: 'failed'; message: string }

export type DiagnosticSeverity = 'info' | 'warning' | 'error'
export type EnvironmentLevel = 'ready' | 'warning' | 'blocking'
export type JobState =
  | 'created'
  | 'preparing'
  | 'capturing'
  | 'analyzing'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'interrupted'
export type CaptureNetwork = 'tcp' | 'udp' | 'all'
export type TargetSource =
  | { mode: 'manual' }
  | {
      mode: 'config'
      config_path: string
      config_sha256: string
      target_index: number
    }
export type FlowNetwork = 'tcp' | 'udp'
export type FlowMatchStatus = 'matched' | 'ambiguous' | 'unmatched' | 'legacy'

export interface EnvironmentRequest {
  tun_interface: string
  physical_interface: string
  chrome_binary: string
  output_root: string
  min_free_bytes?: number | null
}

export interface DiagnosticCheck {
  code: string
  ok: boolean
  severity: DiagnosticSeverity
  message: string
  remediation: string
  details: unknown
}

export interface CompleteIntegrationStatus {
  current_core: string
  tun_enabled: boolean
  service_available: boolean
  configured_tun_device: string
  automatic_tun_device: string
  capture_tun_interface: string
  worker: WorkerManagerState
}

export interface CompleteEnvironmentReport {
  level: EnvironmentLevel
  ok: boolean
  checks: DiagnosticCheck[]
  integration: CompleteIntegrationStatus
}

export interface CaptureOptions {
  capture_packets: boolean
  collect_cdp: boolean
  collect_netlog: boolean
  analyze_after_capture: boolean
  headless: boolean
}

export interface CaptureStartRequest {
  url: string
  domain: string
  duration_seconds: number
  network: CaptureNetwork
  tun_interface: string
  physical_interface: string
  output_root: string
  chrome_binary: string
  wait_load_timeout: number
  run_label: string
  target_source: TargetSource
  options?: Partial<CaptureOptions>
}

export interface TargetConfigEntry {
  index: number
  domain: string
  url: string
  duration_seconds: number
  network: CaptureNetwork
  run_label: string
  wait_load_timeout: number
}

export interface TargetConfigPreview {
  schema_version: 1
  config_path: string
  sha256: string
  targets: TargetConfigEntry[]
  warnings: string[]
}

export interface JobSnapshot {
  job_id: string
  kind: 'capture' | 'analysis'
  state: JobState
  stage: string
  progress: number
  message: string
  cancel_requested: boolean
  cancel_requested_now?: boolean | null
  result?: unknown
  error?: unknown
}

export interface CaptureLockSnapshot {
  locked: boolean
  job_id?: string
  reason?: string
}

export interface SessionListResult {
  sessions: SessionManifest[]
  corrupt: CorruptSession[]
}

export interface CorruptSession {
  session_dir: string
  message: string
}

export interface SessionManifest {
  schema_version: number
  session_id: string
  job_id: string
  state: JobState
  created_at: string
  updated_at: string
  started_at?: string | null
  completed_at?: string | null
  session_dir: string
  target: SessionTarget
  component_versions: ComponentVersions
  artifacts: SessionArtifact[]
  warnings: string[]
  error?: SessionError | null
}

export interface SessionTarget {
  url: string
  domain: string
  source?: TargetSource | null
}

export interface ComponentVersion {
  version: string
  commit: string
}

export interface ComponentVersions {
  traffictracer: ComponentVersion
  mihomo: ComponentVersion
  clash_verge_rev: ComponentVersion
  worker_api: number
}

export interface SessionArtifact {
  name: string
  kind?: string | null
  artifact_id?: string | null
  phase?: 'capture' | 'analysis' | 'diagnostic' | null
  role?: string | null
  generation_id?: string | null
  path: string
  media_type: string
  size_bytes: number
  sha256?: string | null
  created_at?: string | null
}

export interface SessionError {
  code: string
  message: string
  stage?: string | null
}

export interface AnalysisOptions {
  split_pcaps: boolean
  pcap_split_mode: 'none' | 'unique_connections'
  write_flow_index: boolean
  overwrite: boolean
}

export type AnalysisArtifactRole =
  | 'request_index'
  | 'connection_index'
  | 'pcap_index'
  | 'coverage_summary'

export interface CoveragePartition {
  total: number
  matched: number
  ambiguous: number
  unmatched: number
}

export interface LayeredCoverage {
  browser_requests: CoveragePartition
  transport_connections: CoveragePartition
  core_logical_flows: {
    total: number
    with_post_flow: number
    shared: number
    missing_post_flow: number
  }
  unmatched_reasons: Record<string, number>
}

export interface RequestIndexRecord {
  request_id: string
  url: string
  resource_type: string
  relation: string
  connection_id: string | null
  candidate_connection_ids: string[]
  attribution: {
    status: 'matched' | 'ambiguous' | 'unmatched'
    method: string
    confidence: number
    evidence: string[]
    unmatched_reason?: string
  }
}

export interface ConnectionIndexRecord {
  connection_id: string
  protocol: FlowNetwork
  pre_flow: NormalizedFlowTuple
  post_flow: NormalizedFlowTuple | null
  shared: boolean
  request_ids: string[]
  match: {
    status: 'matched' | 'ambiguous' | 'unmatched'
    method: string
    confidence: number
    evidence: string[]
    unmatched_reason?: string
    candidates: Array<{
      connection_id: string
      score: number
      evidence: string[]
    }>
  }
}

export interface AnalysisIndex<T> {
  analysis_generation_id: string
  items: T[]
}

export interface CoverageSummary {
  analysis_generation_id?: string
  coverage: LayeredCoverage
  match_method_counts: Record<string, number>
  coverage_source: string
}

export interface FlowQueryRequest {
  session_id: string
  network: FlowNetwork
  src_ip: string
  src_port: number
  dst_ip: string
  dst_port: number
  offset?: number
  limit?: number
}

export interface FlowQueryResult {
  session_id: string
  offset: number
  limit: number
  total: number
  items: FlowRecord[]
}

export interface FlowRecord {
  schema_version: number
  session_id: string
  flow_id: string
  protocol: FlowNetwork
  pre_flow: NormalizedFlowTuple
  post_flow: NormalizedFlowTuple | null
  shared: boolean
  match: FlowMatch
  request_ids: string[]
  conn_id?: string | null
  outer_conn_id?: string | null
  url?: string | null
  resource_type?: string | null
  relation?: string | null
}

export interface NormalizedFlowTuple {
  network: FlowNetwork
  src_ip: string
  src_port: number
  dst_ip: string
  dst_port: number
  dst_host?: string | null
  complete: boolean
  source: string
  scope: string
  shared: boolean
}

export interface FlowMatch {
  status: FlowMatchStatus
  confidence: number
  candidate_count: number
  reason: string
}

export interface JobProgressEvent {
  job_id: string
  state: JobState
  stage: string
  progress: number
  message: string
  timestamp: string
}

export interface WorkerReadyEvent {
  version: string
  api_version: number
  output_root: string
  recovery: {
    status: 'ok' | 'degraded'
    recovered_sessions: string[]
    terminated_pids: number[]
    skipped_pids: number[]
    errors: string[]
  }
}

export interface WorkerLogEvent {
  level: string
  code?: string
  message: string
  recovery?: WorkerReadyEvent['recovery']
}
