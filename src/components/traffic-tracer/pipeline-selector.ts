import type { PipelineCandidate } from '@/types/traffic-tracer'

export interface PipelineProxyItem {
  name: string
  type: string
  now?: string
  all?: Array<string | { name: string }>
}

export interface PipelineProxyRuntime {
  global?: PipelineProxyItem | null
  groups: PipelineProxyItem[]
  records: Record<string, PipelineProxyItem | undefined>
}

const normalizedType = (value?: string) =>
  (value ?? '').replaceAll(/[-_\s]/g, '').toLowerCase()

export const isPipelineSelector = (proxy?: PipelineProxyItem | null) =>
  normalizedType(proxy?.type) === 'selector'

export const isAutomaticProxyGroup = (proxy?: PipelineProxyItem | null) =>
  ['urltest', 'fallback', 'loadbalance'].includes(normalizedType(proxy?.type))

export function pipelineSelectorGroups(
  runtime?: PipelineProxyRuntime | null,
): PipelineProxyItem[] {
  if (!runtime) return []

  const selectors = runtime.groups.filter(
    (group) => group.name !== 'GLOBAL' && isPipelineSelector(group),
  )
  if (selectors.length > 0) return selectors

  return isPipelineSelector(runtime.global) && runtime.global
    ? [runtime.global]
    : []
}

const memberNames = (group: PipelineProxyItem) =>
  (group.all ?? []).map((member) =>
    typeof member === 'string' ? member : member.name,
  )

export function pipelineCandidateRuntimeIssue(
  runtime: PipelineProxyRuntime | null | undefined,
  selectionGroup: string,
  requestedNode: string,
): string | null {
  if (!runtime) return 'Proxy runtime is unavailable.'

  const group =
    runtime.records[selectionGroup] ??
    runtime.groups.find((item) => item.name === selectionGroup) ??
    (runtime.global?.name === selectionGroup ? runtime.global : undefined)
  if (!group) return 'The selector is absent from the active runtime.'
  if (!isPipelineSelector(group)) {
    return `${selectionGroup} is ${group.type}, not a manual Selector.`
  }
  if (!memberNames(group).includes(requestedNode)) {
    return `${requestedNode} is not selectable from ${selectionGroup}.`
  }

  const requested = runtime.records[requestedNode]
  if (!requested) return `${requestedNode} is absent from the active runtime.`
  if (isAutomaticProxyGroup(requested)) {
    return `${requestedNode} is an automatic ${requested.type} group; choose a concrete node.`
  }
  return null
}

export function pipelineCandidateIdentity(
  candidate: Pick<
    PipelineCandidate,
    'profile_uid' | 'selection_group' | 'requested_node'
  >,
): string {
  return [
    candidate.profile_uid,
    candidate.selection_group,
    candidate.requested_node,
  ].join('\u0000')
}
