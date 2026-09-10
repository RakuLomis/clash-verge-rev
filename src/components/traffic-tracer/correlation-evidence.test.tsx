import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type {
  LayeredCoverage,
  RequestIndexRecord,
} from '@/types/traffic-tracer'

import { CorrelationEvidence } from './correlation-evidence'

const partition = { total: 5, matched: 4, ambiguous: 1, unmatched: 0 }
const logical = {
  total: 5,
  with_post_flow: 5,
  shared: 0,
  missing_post_flow: 0,
  unexpected_missing: 0,
}
const coverage: LayeredCoverage = {
  browser_requests: partition,
  transport_connections: partition,
  core_logical_flows: logical,
  unmatched_reasons: {},
  page_attributed: {
    browser_requests: partition,
    transport_connections: partition,
    logical_flows: logical,
    unmatched_reasons: {},
  },
  capture_global: {
    core_logical_flows: { ...logical, capture_tail_unattributed: 2 },
    unmatched_reasons: {},
  },
}

describe('correlation evidence scope', () => {
  it('keeps page ambiguity and global tail evidence separate', () => {
    render(<CorrelationEvidence coverage={coverage} requests={[]} />)
    expect(
      screen.getByText(/1 ambiguous requests · 2 background tail flows/),
    ).toBeTruthy()
    expect(
      screen.getByText(/Page-attributed unexpected missing endpoints: 0/),
    ).toBeTruthy()
    expect(screen.getByText(/Showing 0 examples from loaded/)).toBeTruthy()
  })

  it('shows observed candidate evidence without selecting a winner', () => {
    const request: RequestIndexRecord = {
      request_id: 'r1',
      url: 'https://example.test/',
      resource_type: 'Document',
      relation: 'unknown',
      connection_id: null,
      candidate_connection_ids: ['c1', 'c2'],
      attribution: {
        status: 'ambiguous',
        method: 'endpoint',
        confidence: 0,
        unmatched_reason: 'ambiguous_response_endpoint',
        evidence: ['remote_endpoint'],
      },
    }
    render(<CorrelationEvidence coverage={coverage} requests={[request]} />)
    expect(screen.getByText(/Candidates: c1, c2/)).toBeTruthy()
    expect(screen.getByText(/Evidence: remote_endpoint/)).toBeTruthy()
    expect(screen.getByText(/Showing 1 examples/)).toBeTruthy()
  })

  it('does not invent evidence without a summary', () => {
    const { container } = render(<CorrelationEvidence requests={[]} />)
    expect(container.textContent).toBe('')
  })
})
