import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, renderHook, waitFor } from '@testing-library/react'
import type { PropsWithChildren } from 'react'
import { afterEach, expect, it, vi } from 'vitest'

vi.mock('@/services/cmds', () => ({
  getTrafficTracerCaptureLock: vi.fn(async () => ({ locked: false })),
  getTrafficTracerEnvironment: vi.fn(async () => ({ status: 'ok' })),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

import { getTrafficTracerEnvironment } from '@/services/cmds'

import { useTrafficTracerWorker } from './use-traffic-tracer-worker'

const request = {
  output_root: '/tmp/test-workspace',
  tun_interface: 'Meta',
  physical_interface: 'eth0',
  chrome_binary: '/usr/bin/chromium',
}
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function harness() {
  const client = new QueryClient()
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  return { client, wrapper }
}

it('does not start diagnostics on mount, completion, invalidation or remount', async () => {
  const { client, wrapper } = harness()
  const hook = renderHook(
    ({ paused }) => useTrafficTracerWorker(request, true, paused),
    {
      wrapper,
      initialProps: { paused: true },
    },
  )
  await waitFor(() => expect(hook.result.current.captureLock).toBeDefined())
  hook.rerender({ paused: false })
  await act(async () => {
    await client.invalidateQueries({
      queryKey: ['trafficTracer', 'environment'],
    })
  })
  expect(getTrafficTracerEnvironment).not.toHaveBeenCalled()
  await act(async () => {
    await hook.result.current.checkEnvironment(request)
  })
  expect(getTrafficTracerEnvironment).toHaveBeenCalledTimes(1)
  hook.unmount()
  const reopened = renderHook(() => useTrafficTracerWorker(request), {
    wrapper,
  })
  await waitFor(() => expect(reopened.result.current.environment).toBeDefined())
  expect(getTrafficTracerEnvironment).toHaveBeenCalledTimes(1)
  reopened.unmount()
  client.clear()
})

it('rejects explicit checks while the pipeline is active', async () => {
  const { client, wrapper } = harness()
  const hook = renderHook(() => useTrafficTracerWorker(request, true, true), {
    wrapper,
  })
  await expect(hook.result.current.checkEnvironment(request)).rejects.toThrow(
    'capture is active',
  )
  expect(getTrafficTracerEnvironment).not.toHaveBeenCalled()
  hook.unmount()
  client.clear()
})
