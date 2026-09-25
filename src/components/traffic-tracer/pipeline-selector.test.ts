import { describe, expect, it } from 'vitest'

import {
  pipelineCandidateIdentity,
  pipelineCandidateRuntimeIssue,
  pipelineSelectorGroups,
  type PipelineProxyRuntime,
} from './pipeline-selector'

const runtime: PipelineProxyRuntime = {
  global: { name: 'GLOBAL', type: 'Selector', now: 'Yu-VPS' },
  groups: [
    {
      name: 'Yu-VPS',
      type: 'Selector',
      now: 'out-vless-tls',
      all: [
        { name: '自动选择' },
        { name: '故障转移' },
        { name: 'out-vless-tls' },
        { name: 'out-trojan-tls' },
      ],
    },
    {
      name: '自动选择',
      type: 'URLTest',
      now: 'out-vless-tls',
      all: [{ name: 'out-vless-tls' }, { name: 'out-trojan-tls' }],
    },
    {
      name: '故障转移',
      type: 'Fallback',
      now: 'out-vless-tls',
      all: [{ name: 'out-vless-tls' }, { name: 'out-trojan-tls' }],
    },
  ],
  records: {
    'Yu-VPS': {
      name: 'Yu-VPS',
      type: 'Selector',
      now: 'out-vless-tls',
      all: ['自动选择', '故障转移', 'out-vless-tls', 'out-trojan-tls'],
    },
    自动选择: {
      name: '自动选择',
      type: 'URLTest',
      now: 'out-vless-tls',
      all: ['out-vless-tls', 'out-trojan-tls'],
    },
    故障转移: {
      name: '故障转移',
      type: 'Fallback',
      now: 'out-vless-tls',
      all: ['out-vless-tls', 'out-trojan-tls'],
    },
    'out-vless-tls': { name: 'out-vless-tls', type: 'Vless' },
    'out-trojan-tls': { name: 'out-trojan-tls', type: 'Trojan' },
  },
}

describe('TrafficTracer pipeline selector resolution', () => {
  it('selects the manual selector instead of keyword-matching URLTest groups', () => {
    expect(pipelineSelectorGroups(runtime).map((group) => group.name)).toEqual([
      'Yu-VPS',
    ])
  })

  it('accepts distinct concrete nodes from the same selector', () => {
    expect(
      pipelineCandidateRuntimeIssue(runtime, 'Yu-VPS', 'out-vless-tls'),
    ).toBeNull()
    expect(
      pipelineCandidateRuntimeIssue(runtime, 'Yu-VPS', 'out-trojan-tls'),
    ).toBeNull()
    expect(
      pipelineCandidateIdentity({
        profile_uid: 'yu-profile',
        selection_group: 'Yu-VPS',
        requested_node: 'out-vless-tls',
      }),
    ).not.toBe(
      pipelineCandidateIdentity({
        profile_uid: 'yu-profile',
        selection_group: 'Yu-VPS',
        requested_node: 'out-trojan-tls',
      }),
    )
  })

  it('rejects automatic groups as selectors or requested nodes', () => {
    expect(
      pipelineCandidateRuntimeIssue(runtime, '自动选择', 'out-vless-tls'),
    ).toContain('not a manual Selector')
    expect(
      pipelineCandidateRuntimeIssue(runtime, 'Yu-VPS', '自动选择'),
    ).toContain('automatic URLTest group')
    expect(
      pipelineCandidateRuntimeIssue(runtime, 'Yu-VPS', '故障转移'),
    ).toContain('automatic Fallback group')
  })

  it('falls back to GLOBAL only when no concrete selector exists', () => {
    expect(
      pipelineSelectorGroups({ ...runtime, groups: [] }).map(
        (group) => group.name,
      ),
    ).toEqual(['GLOBAL'])
  })
})
