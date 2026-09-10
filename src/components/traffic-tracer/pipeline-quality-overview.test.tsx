import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, expect, it } from 'vitest'

import type { PipelineManifest } from '@/types/traffic-tracer'

import PipelineQualityOverview from './pipeline-quality-overview'

afterEach(cleanup)
const plane = {
  passed: 64,
  degraded: 0,
  failed: 0,
  indeterminate: 0,
  not_applicable: 0,
}
const fixture = () =>
  ({
    pipeline_id: 'fixture',
    updated_at: 'now',
    aggregate: {
      pipeline_id: 'fixture',
      updated_at: 'now',
      planned_cells: 192,
      terminal_cells: 192,
      candidates: [1, 3, 2].map((id, index) => ({
        candidate_ordinal: id,
        cells_planned: 64,
        cells_terminal: 64,
        completed: [60, 58, 55][index],
        degraded: [4, 6, 9][index],
        failed: 0,
        interrupted: 0,
        cancelled: 0,
        capture_integrity: plane,
        correlation: plane,
        application: {
          ...plane,
          passed: [60, 58, 55][index],
          degraded: [4, 6, 9][index],
        },
      })),
    },
  }) as PipelineManifest

it('shows whole-pipeline totals independently of the last candidate ordinal', () => {
  render(<PipelineQualityOverview pipeline={fixture()} />)
  expect(
    screen.getByText(/Tasks finished 192\/192 · Nodes finished 3\/3/),
  ).toBeInTheDocument()
  expect(screen.getByText(/173 fully passed · 19 degraded/)).toBeInTheDocument()
})
it('does not count a partially completed candidate as finished', () => {
  const p = fixture()
  p.aggregate!.terminal_cells = 191
  p.aggregate!.candidates[2].cells_terminal = 63
  render(<PipelineQualityOverview pipeline={p} />)
  expect(screen.getByText(/Nodes finished 2\/3/)).toBeInTheDocument()
})
it('rejects stale summaries when switching history or refreshing a revision', () => {
  const p = fixture()
  p.aggregate!.updated_at = 'old'
  const view = render(<PipelineQualityOverview pipeline={p} />)
  expect(screen.getByText(/awaiting current summary/)).toBeInTheDocument()
  p.aggregate!.updated_at = 'now'
  p.aggregate!.pipeline_id = 'other'
  view.rerender(<PipelineQualityOverview pipeline={p} />)
  expect(screen.getByText(/awaiting current summary/)).toBeInTheDocument()
})
