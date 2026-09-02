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
  pcap_split_mode: 'none' | 'unique_connections'
  cache_mode: 'cold' | 'warm'
  proxy_protocol_mode: 'strict_single' | 'observe'
  expected_proxy_protocol: string
  proxy_selection_group: string
}

export interface PlaybackPolicy {
  provider: 'youtube'
  ad_policy: 'click_visible_skip'
  desired_primary_seconds: number
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
  page_type: string
  target_source: TargetSource
  options?: Partial<CaptureOptions>
  playback?: PlaybackPolicy
}

export interface TargetConfigEntry {
  index: number
  domain: string
  url: string
  duration_seconds: number
  network: CaptureNetwork
  run_label: string
  wait_load_timeout: number
  page_type: string
  playback?: PlaybackPolicy
}

export interface TargetConfigPreview {
  schema_version: 1
  config_path: string
  sha256: string
  targets: TargetConfigEntry[]
  warnings: string[]
  suggested_output_root: string | null
}

export interface JobSnapshot {
  job_id: string
  kind: 'capture' | 'analysis' | 'batch'
  state: JobState
  stage: string
  progress: number
  message: string
  cancel_requested: boolean
  interrupt_requested?: boolean
  cancel_requested_now?: boolean | null
  interrupt_requested_now?: boolean | null
  result?: unknown
  error?: unknown
}

export interface BatchStartRequest {
  config_path: string
  config_sha256: string
  targets: TargetConfigEntry[]
  tun_interface: string
  physical_interface: string
  output_root: string
  chrome_binary: string
  options: CaptureOptions
  fail_fast: boolean
}

export interface PipelineCandidate {
  profile_uid: string
  profile_fingerprint: string
  selection_group: string
  requested_node: string
}

export interface PipelineStartRequest {
  batch: BatchStartRequest
  candidates: PipelineCandidate[]
  continue_on_run_failure: boolean
}

export type PipelineState =
  | 'created'
  | 'validating'
  | 'running'
  | 'interrupted'
  | 'completed'
  | 'completed_with_errors'
  | 'failed'
  | 'cancelled'
  | 'restoring'
  | 'restore_failed'

export type PipelineStage =
  | 'queued'
  | 'activating_profile'
  | 'waiting_controller'
  | 'selecting_proxy'
  | 'draining_connections'
  | 'preflight'
  | 'running_batch'
  | 'verifying_protocol'
  | 'checkpoint'
  | 'restoring'
  | 'finished'

export interface PipelineRun {
  ordinal: number
  run_id: string
  profile_uid: string
  profile_fingerprint: string
  selection_group: string
  requested_node: string
  state:
    | 'pending'
    | 'running'
    | 'completed'
    | 'degraded'
    | 'failed'
    | 'interrupted'
    | 'skipped'
    | 'cancelled'
  stage: PipelineStage
  resolved_chain: string[]
  resolved_leaf: string | null
  expected_protocol: string
  observed_protocol: string
  batch_id: string | null
  output_path: string
  error: { code: string; message: string } | null
  resume_attempt: number
  started_at: string | null
  completed_at: string | null
}

export interface PipelineListEntry {
  pipeline_id: string
  output_root: string
  state: PipelineState
  updated_at: string
  completed_runs: number
  total_runs: number
}

export interface PipelineManifest {
  schema_version: 1
  pipeline_id: string
  state: PipelineState
  stage: PipelineStage
  created_at: string
  updated_at: string
  output_root: string
  config: { path: string; sha256: string }
  targets: TargetConfigEntry[]
  execution: Record<string, unknown>
  policy: {
    continue_on_run_failure: boolean
    restore_original_state: true
  }
  current_run_index: number | null
  runs: PipelineRun[]
  restore: {
    profile_uid: string | null
    selections: Array<{ group: string; node: string }>
    state: 'pending' | 'not_required' | 'restoring' | 'restored' | 'failed'
    error: { code: string; message: string } | null
  }
}

export type BatchState =
  | 'created'
  | 'running'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'interrupted'

export interface BatchChild {
  target_index: number
  state:
    | 'pending'
    | 'running'
    | 'completed'
    | 'failed'
    | 'cancelled'
    | 'interrupted'
  session_id: string | null
  error: { code: string; message: string } | null
}

export interface BatchManifest {
  schema_version: 1
  batch_id: string
  state: BatchState
  stage:
    | 'queued'
    | 'capture'
    | 'quiescence'
    | 'analysis'
    | 'checkpoint'
    | 'finished'
  created_at: string
  updated_at: string
  output_root: string
  config: { path: string; sha256: string }
  targets: TargetConfigEntry[]
  current_index: number | null
  children: BatchChild[]
  fail_fast: boolean
  cancel_requested: boolean
  resume: { attempt: number; next_index: number; resumed_at: string | null }
}

export interface BatchStatusResult {
  batch: BatchManifest
  job: JobSnapshot | null
}

export interface BatchListResult {
  batches: BatchManifest[]
  corrupt: Array<{ path: string; message: string }>
}

export interface CaptureLockSnapshot {
  locked: boolean
  owner_kind?: 'job' | 'pipeline' | string
  job_id?: string
  reason?: string
}

export interface SessionListResult {
  sessions: SessionSummary[]
  corrupt: CorruptSession[]
  offset: number
  limit: number
  total: number
  has_more: boolean
}

export type PacketSplitStatus =
  | 'unsplit'
  | 'complete'
  | 'complete_empty'
  | 'partial'
  | 'stale'
  | 'raw_missing'
  | 'ineligible'

export interface PacketSplitInspection {
  status: PacketSplitStatus
  reason: string
  connection_count: number
  runnable_missing: boolean
  runnable_repair: boolean
}

export interface PacketSplitPreview {
  scope: SessionScope
  total: number
  counts: Partial<Record<PacketSplitStatus, number>>
  missing_only: number
  repair_incomplete: number
  sessions: Array<PacketSplitInspection & { session_id: string; url: string }>
  corrupt: CorruptSession[]
}

export interface SessionScope {
  scope_id: string
  display_name: string
  directory: string
  kind: 'capture_group' | 'legacy_session'
  created_at: string | null
  exists: boolean
}

export interface ScopedSessionListResult extends SessionListResult {
  scope: SessionScope
}

export type SessionScopeSelector =
  | { path: string; job_id?: never; batch_id?: never }
  | { path?: never; job_id: string; batch_id?: never }
  | { path?: never; job_id?: never; batch_id: string }

export interface CorruptSession {
  session_dir: string
  message: string
}

export interface SessionSummary {
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
  artifact_count: number
  warning_count: number
  quality_state: string | null
  capture_global_quality_state: string | null
  analysis_integrity_state?: string | null
  network_outcome_state?: string | null
  scenario_outcome_state?: string | null
  coverage: Record<string, unknown> | null
  packet_split?: PacketSplitInspection
  error?: SessionError | null
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
  non_network?: number
}

export interface LogicalFlowCoverage {
  total: number
  with_post_flow: number
  shared: number
  missing_post_flow: number
  not_applicable_local_endpoint?: number
  not_applicable_outcome?: number
  explicit_no_socket?: number
  failed_before_socket?: number
  local_not_applicable?: number
  unexpected_missing?: number
  capture_tail_unattributed?: number
}

export interface LayeredCoverage {
  browser_requests: CoveragePartition
  transport_connections: CoveragePartition
  core_logical_flows: LogicalFlowCoverage
  page_attributed?: {
    browser_requests: CoveragePartition
    transport_connections: CoveragePartition
    logical_flows: LogicalFlowCoverage
    unmatched_reasons: Record<string, number>
  }
  capture_global?: {
    core_logical_flows: LogicalFlowCoverage
    unmatched_reasons: Record<string, number>
    attribution_scopes?: Record<string, number>
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
  network_observation?:
    | 'network'
    | 'disk_cache'
    | 'service_worker'
    | 'prefetch_cache'
    | 'browser_internal'
    | 'local_endpoint'
    | 'unknown'
  attribution: {
    status: 'matched' | 'ambiguous' | 'unmatched'
    method: string
    confidence: number
    evidence: string[]
    unmatched_reason?: string
  }
}

export interface FlowTerminal {
  status: string
  stage: string
  error: string
  bytes_up: number
  bytes_down: number
  duration_ms: number
  error_class?: string
  error_class_source?: 'core_explicit' | 'legacy_inferred' | 'unavailable'
}

export interface CarrierBindingRecord {
  carrier_id: string
  status: 'shared_bound' | 'exclusive_bound'
  mode: 'shared' | 'exclusive'
  relation: string
  generation: number
  protocol: string
  physical_paths: NormalizedFlowTuple[]
}

export interface ConnectionIndexRecord {
  connection_id: string
  protocol: FlowNetwork
  application_protocol?: 'unknown' | 'h2' | 'h3'
  attempted_protocols?: string[]
  timing?: {
    first_observed: number | null
    last_observed: number | null
    first_observed_utc?: number | null
    last_observed_utc?: number | null
  }
  pre_flow: NormalizedFlowTuple
  terminal?: FlowTerminal
  netlog_source_id?: number
  mihomo_connection_id?: string
  post_flow: NormalizedFlowTuple | null
  carrier_binding?: CarrierBindingRecord
  shared: boolean
  sharing?: {
    request_multiplexed: boolean
    post_flow_shared: boolean
    outer_connection_reused: boolean
  }
  egress?: {
    mode: 'direct' | 'proxy' | 'unknown'
    outcome?:
      | 'direct'
      | 'proxy'
      | 'rejected'
      | 'rejected_drop'
      | 'internal_dns'
      | 'pass'
      | 'compatible'
      | 'unknown'
    policy: string | null
    selection_chain: string[]
    selected_node: string | null
    selected_type: string | null
    evidence: string
  }
  request_ids: string[]
  primary_url: string | null
  urls: string[]
  attribution_scope?:
    | 'page_attributed'
    | 'browser_background'
    | 'capture_unattributed'
    | 'local_internal'
  attribution_evidence?: string[]
  post_flow_disposition?:
    | 'with_post_flow'
    | 'explicit_no_socket'
    | 'failed_before_socket'
    | 'local_not_applicable'
    | 'unexpected_missing'
  match: {
    status: 'matched' | 'ambiguous' | 'unmatched'
    method: string
    confidence: number
    evidence: string[]
    unmatched_reason?: string
    candidate_count?: number
    candidates_truncated?: boolean
    time_evidence?: {
      available: boolean
      delta_ms: number | null
      source: 'netlog_tick_offset_to_utc' | 'same_clock' | 'unavailable'
    }
    candidates: Array<{
      connection_id: string
      score: number
      evidence: string[]
      time_delta_ms?: number | null
      time_source?: 'netlog_tick_offset_to_utc' | 'same_clock' | 'unavailable'
    }>
  }
}

export interface AnalysisIndex<T> {
  analysis_generation_id: string
  items: T[]
}

export interface AnalysisWarning {
  code: string
  count: number
  message: string
  scope?: 'page_attributed' | 'capture_global'
  severity?: 'info' | 'warning' | 'error'
  affects_page_quality?: boolean
}

export interface AnalysisPageQuality {
  request_attribution: {
    eligible: number
    matched: number
    ambiguous: number
    unmatched: number
  }
  transport_correlation: CoveragePartition
  egress_establishment: {
    total: number
    established: number
    failed_before_socket: number
    unavailable: number
    not_applicable_local_endpoint?: number
    not_applicable_outcome?: number
  }
  pcap_extraction: {
    requested: boolean
    total: number
    applicable?: number
    post_applicable?: number
    pre_success: number
    post_success: number
    complete_pairs: number
    post_not_applicable?: number
    post_not_requested?: number
  }
}

export interface AnalysisQuality extends AnalysisPageQuality {
  page_attributed?: AnalysisPageQuality
  capture_global?: {
    logical_flows: Partial<LogicalFlowCoverage> & {
      total: number
      with_post_flow: number
      missing_post_flow: number
      errors: number
    }
  }
}

export interface TraceSnapshotEntry {
  source: 'mihomo_barrier' | 'legacy_unbounded'
  cutoff_event_seq: number | null
  barrier_ts: string
  barrier_session_id: string
  late_event_count: number
  max_observed_event_seq: number
  late_event_types?: Record<string, number>
  max_late_delay_ms?: number
  barrier_verified?: boolean
}

export interface TraceSnapshotSummary {
  source: 'mihomo_barrier' | 'legacy_unbounded' | 'mixed'
  trace_count: number
  late_event_count: number
  late_event_types?: Record<string, number>
  max_late_delay_ms?: number
  traces: TraceSnapshotEntry[]
}

export interface AnalysisStorageSummary {
  capture_bytes: number
  raw_packet_capture_bytes: number
  netlog_bytes: number
  mihomo_trace_bytes: number
  capture_metadata_bytes: number
  analysis_result_bytes_before_summary: number
  compression: 'none'
}

export interface AnalysisNetworkOutcome {
  state:
    | 'healthy'
    | 'partial_failure'
    | 'failed'
    | 'not_applicable'
    | 'indeterminate'
  applicable: number
  established: number
  failed_before_socket: number
  explicit_no_socket: number
  local_not_applicable: number
  unexpected_missing: number
  failed_requests?: number
}

export interface CoverageSummary {
  analysis_generation_id?: string
  coverage: LayeredCoverage
  match_method_counts: Record<string, number>
  coverage_source: string
  carrier_bindings?: {
    logical_proxy_flows: number
    bound_logical_flows: number
    missing_binding: number
    exclusive_socket_count: number
    shared_bound_logical_flows: number
    shared_carrier_count: number
    shared_carrier_max_fan_out: number
    shared_carrier_fan_out: Record<string, number>
    physical_carriers_observed: number
  }
  proxy_protocol?: {
    mode?: string
    expected_protocol: string
    selected_protocols?: string[]
    selection_group?: string
    selected_scope?: Record<string, unknown>
    inventory_protocols?: string[]
    observed_protocols: string[]
    consistency: string
    proxy_dial_events?: number
  }
  inbound?: {
    mode?: string
    interface?: string
    expected_core_name?: string
    observed_names?: Record<string, number>
    mismatched_flows?: number
    loopback_flows?: number
    consistency?: string
  }
  quality_state?: 'passed' | 'degraded' | 'failed'
  capture_global_quality_state?: 'passed' | 'degraded' | 'failed'
  quality?: AnalysisQuality
  analysis_integrity?: {
    page_attributed: { state: 'passed' | 'degraded' | 'failed' }
    capture_global: { state: 'passed' | 'degraded' | 'failed' }
  }
  network_outcome?: {
    page_attributed: AnalysisNetworkOutcome
    capture_global: AnalysisNetworkOutcome
  }
  browser_request_failures?: {
    total_requests: number
    failed_occurrences: number
    canceled_occurrences: number
    recovered_occurrences: number
    unrecovered_occurrences: number
    by_reason: Record<string, number>
    by_relation: Record<string, number>
    recovered_by_reason: Record<string, number>
    recovery_evidence: 'later_successful_occurrence_same_url'
  }
  warnings?: AnalysisWarning[]
  trace_snapshot?: TraceSnapshotSummary
  storage?: AnalysisStorageSummary
  playback?: {
    provider: 'youtube'
    ad_policy?: 'click_visible_skip'
    observation_window_seconds: number
    observed_total_seconds?: number
    desired_primary_seconds: number
    primary_content_seconds: number
    primary_content_observed?: boolean
    primary_goal_met: boolean
    quality: 'good' | 'degraded' | 'unavailable' | 'unknown'
    reason?: string | null
    ad_observed?: boolean
    skippable_ad_observed?: boolean
    skip_attempts?: number
    skip_confirmed?: boolean
    play_attempts?: number
    recovery_attempts?: number
    reload_command_sent?: boolean
    interaction_errors?: number
    recovery_errors?: number
    phase_seconds?: Record<string, number>
    diagnostics?: {
      counts?: Record<string, number>
      last_observation?: Record<string, unknown>
      first_seen_at_seconds?: {
        player?: number | null
        video?: number | null
      }
    }
  }
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
  carrier_binding?: CarrierBindingRecord
  carrier_state?:
    | 'exclusive_bound'
    | 'shared_bound'
    | 'not_applicable'
    | 'failed_before_carrier'
    | 'observation_missing'
  inbound_name?: string
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
  timing?: {
    job_elapsed_ms: number
    stage_elapsed_ms: number
    operation: string
    operation_elapsed_ms: number
    completed_stage?: string
    completed_stage_duration_ms?: number
    completed_operation?: string
    completed_operation_duration_ms?: number
  }
}

export interface WorkerReadyEvent {
  version: string
  api_version: number
  output_root: string
  recovery: {
    status: 'ok' | 'degraded'
    recovered_sessions?: string[]
    terminated_pids?: number[]
    skipped_pids?: number[]
    errors?: string[]
    recovered_session_count?: number
    terminated_pid_count?: number
    skipped_pid_count?: number
    error_count?: number
    summary_only?: boolean
  }
}

export interface WorkerLogEvent {
  level: string
  code?: string
  message: string
  recovery?: WorkerReadyEvent['recovery']
  timing?: {
    operation: string
    duration_ms: number
    catalog?: {
      operation?: string
      duration_ms?: number
      [key: string]: unknown
    }
  }
}
