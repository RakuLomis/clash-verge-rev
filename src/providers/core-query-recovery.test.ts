import { describe, expect, it } from 'vitest'

import {
  CORE_ERROR_REFETCH_INTERVAL_MS,
  CORE_REFRESH_QUERY_KEYS,
  coreQueryRecoveryInterval,
} from './core-query-recovery'

describe('core query recovery', () => {
  it('polls only while a core query is in the error state', () => {
    expect(coreQueryRecoveryInterval('error')).toBe(
      CORE_ERROR_REFETCH_INTERVAL_MS,
    )
    expect(coreQueryRecoveryInterval('pending')).toBe(false)
    expect(coreQueryRecoveryInterval('success')).toBe(false)
  })

  it('refreshes every query backed by the active core controller', () => {
    expect(CORE_REFRESH_QUERY_KEYS).toEqual([
      'getProxies',
      'getVersion',
      'getClashConfig',
      'getProxyProviders',
      'getRules',
      'getRuleProviders',
    ])
  })
})
