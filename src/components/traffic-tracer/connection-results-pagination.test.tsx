import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, it } from 'vitest'

import type {
  ConnectionIndexRecord,
  RequestIndexRecord,
} from '@/types/traffic-tracer'

import { TrafficTracerConnectionResults } from './connection-results'

afterEach(cleanup)
const requests: RequestIndexRecord[] = Array.from(
  { length: 201 },
  (_, index) => ({
    request_id: `request-${index}`,
    url: `https://example.test/request/${index}`,
    resource_type: 'Document',
    relation: 'main',
    connection_id: `conn-${index}`,
    candidate_connection_ids: [],
    attribution: {
      status: 'matched',
      method: 'fixture',
      confidence: 1,
      evidence: [],
    },
  }),
)
const connections: ConnectionIndexRecord[] = requests.map((request, index) => ({
  connection_id: `conn-${index}`,
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
  request_ids: [request.request_id],
  primary_url: `https://example.test/connection/${index}`,
  urls: [request.url],
  match: {
    status: 'matched',
    method: 'fixture',
    confidence: 1,
    evidence: [],
    candidates: [],
  },
}))
it('bounds rendered rows without discarding access to the last records', () => {
  render(
    <TrafficTracerConnectionResults
      requests={requests}
      connections={connections}
    />,
  )
  expect(screen.getAllByRole('row').length).toBeLessThanOrEqual(102)
  expect(screen.getByText('Browser requests (201)')).toBeInTheDocument()
  expect(screen.queryByText(requests[200].url)).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Last request page' }))
  expect(screen.getByText(requests[200].url)).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Last connection page' }))
  expect(screen.getByText(connections[200].primary_url!)).toBeInTheDocument()
})
it('clamps pages when an index refresh has fewer records', () => {
  const view = render(
    <TrafficTracerConnectionResults
      requests={requests}
      connections={connections}
    />,
  )
  fireEvent.click(screen.getByRole('button', { name: 'Last request page' }))
  fireEvent.click(screen.getByRole('button', { name: 'Last connection page' }))
  view.rerender(
    <TrafficTracerConnectionResults
      requests={requests.slice(0, 1)}
      connections={connections.slice(0, 1)}
    />,
  )
  expect(screen.getByText(requests[0].url)).toBeInTheDocument()
  expect(screen.getByText(connections[0].primary_url!)).toBeInTheDocument()
})
