export const CORE_ERROR_REFETCH_INTERVAL_MS = 5_000

export const coreQueryRecoveryInterval = (status: string) =>
  status === 'error' ? CORE_ERROR_REFETCH_INTERVAL_MS : false

export const CORE_REFRESH_QUERY_KEYS = [
  'getProxies',
  'getVersion',
  'getClashConfig',
  'getProxyProviders',
  'getRules',
  'getRuleProviders',
] as const
